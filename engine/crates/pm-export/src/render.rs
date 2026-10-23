//! Exports and agent stills rendered in the job engine (plan section 17,
//! "Pipeline"; section 20, "Frames"):
//!
//! ```text
//! capability check → evaluate + render (pm-render, float) → readback
//!   ─(bounded queue)→ encoder input (matrix, siting-aware chroma, dither)
//!   + offline mix (pm-audio) → encode → mux → verify → atomic move
//! ```
//!
//! Rendering runs on the calling thread (the job engine's worker, which owns
//! the [`Renderer`]); conversion, mixing, encoding, and muxing run on a second
//! thread with one frame queued between them, so the GPU renders frame n + 1
//! while frame n is encoded.
//!
//! Every GPU submission goes through a [`GpuGate`] (contract 13): without
//! playback the gate admits whole frames; while the interactive engine plays it
//! admits bands of [`BAND_ROWS`]-row units sized to the slicing budget, rendered
//! as regions of interest (pm-render renders a region bit for bit equal to the
//! same rows of the full frame).
//!
//! Capability routing (section 26, Phase 2): a composition that pm-render cannot
//! draw, media the engine cannot open, or audio pm-audio cannot decode fails
//! with [`JobError::Unsupported`] before anything is written, so the host sends
//! the export to the legacy exporter instead.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use pm_model::{Document, Node, NodeKind};
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{PixelRect, Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, AudioCoding, Container as WireContainer, ExportFormat,
    ExportSettings, ProResProfile as WireProRes, WebmCodec,
};
use pm_render::{RenderError, RenderRequest, Renderer, VideoEnd};
use pm_time::Rate;

use crate::Error;
use crate::convert::WorkingFrame;
use crate::pipeline::{Pipeline, Sink};
use crate::settings::{
    AlphaOutput, AudioCodec, AudioSpec, Codec, ColorTags, Container, Dither, ExportSpec, Overwrite,
    ProResProfile, WorkingEncoding,
};

/// Output rows per slicing unit while the interactive engine plays.
pub const BAND_ROWS: u32 = 16;

/// Why a render job did not produce its output.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// The engine cannot render this faithfully yet; the host routes the job
    /// to the legacy engine (section 26, Phase 2 capability routing).
    #[error("the new engine cannot render this yet: {}", .0.join("; "))]
    Unsupported(Vec<String>),
    #[error("cancelled")]
    Cancelled,
    #[error("invalid job: {0}")]
    Invalid(String),
    #[error("render failed: {0}")]
    Render(String),
    #[error("audio failed: {0}")]
    Audio(String),
    /// Blender is missing, failed, or did not deliver a surface (contract
    /// 11: `blenderMissing`, `blenderFailed`).
    #[error("{0}")]
    Blender(pm_blender::BlenderError),
    #[error(transparent)]
    Export(#[from] Error),
}

impl From<RenderError> for JobError {
    fn from(e: RenderError) -> Self {
        match e {
            RenderError::Unsupported(list) => {
                JobError::Unsupported(list.iter().map(ToString::to_string).collect())
            }
            other => JobError::Render(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// Asset id → file, resolved by host services (contract 14 `mediaRead`
/// handles) before the job starts. Assets without a file are offline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaPaths {
    pub paths: BTreeMap<String, PathBuf>,
}

impl MediaPaths {
    pub fn insert(&mut self, asset: impl Into<String>, path: PathBuf) {
        self.paths.insert(asset.into(), path);
    }
}

impl pm_render::AssetResolver for MediaPaths {
    fn path(&self, asset: &str, _record: Option<&Node>) -> Option<PathBuf> {
        self.paths.get(asset).cloned()
    }
}

impl pm_audio::AssetResolver for MediaPaths {
    fn resolve(&self, asset: &str) -> Option<PathBuf> {
        self.paths.get(asset).cloned()
    }
}

/// The asset ids a document records (`root.assets` keys): what a job asks the
/// host to resolve before rendering.
pub fn asset_ids(document: &Document) -> Vec<String> {
    let Some(assets) = pm_eval::node::member(document.root(), "assets") else {
        return vec![];
    };
    match assets.kind() {
        NodeKind::Object(map) => map.keys().map(|k| k.to_string_lossy()).collect(),
        _ => vec![],
    }
}

/// Each asset of `document` with the digest of its record (canonical JSON):
/// a relink or a replaced file changes the record, so hosts that keep
/// resolved paths can tell when to resolve an asset again.
pub fn asset_records(document: &Document) -> Vec<(String, [u8; 32])> {
    let assets = pm_eval::node::member(document.root(), "assets");
    asset_ids(document)
        .into_iter()
        .map(|asset| {
            let digest = pm_eval::node::member(assets, &asset)
                .map_or([0; 32], pm_model::hash::canonical_digest);
            (asset, digest)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GPU time slicing
// ---------------------------------------------------------------------------

/// The job engine's submission gate (contract 13). The export and still
/// renderers ask it before every GPU submission and report what each took.
pub trait GpuGate: Send + Sync {
    /// Blocks until GPU work may be submitted and returns how many of the
    /// `units` remaining units of `pass` to submit now (1 to `units`). Returns
    /// `None` when `cancel` is set while waiting.
    fn admit(&self, pass: u64, units: u32, cancel: &AtomicBool) -> Option<u32>;
    /// The submission admitted last completed: `units` units of `pass` in
    /// `measured_us` of GPU time (submission to completion).
    fn finished(&self, pass: u64, units: u32, measured_us: f64);
    /// [`finished`](GpuGate::finished), saying whether `measured_us` is the
    /// submission's GPU time from timestamps (`true`) or wall time from
    /// submission to completion, which also counts waits behind other GPU
    /// clients (`false`). The renderer calls this one.
    fn finished_timed(&self, pass: u64, units: u32, measured_us: f64, gpu_timed: bool) {
        let _ = gpu_timed;
        self.finished(pass, units, measured_us);
    }
    /// The submission admitted last never completed (the renderer returned an
    /// error or panicked): releases the admission without measuring it.
    /// Every [`admit`](GpuGate::admit) that returns `Some` is followed by
    /// exactly one `finished` or `abandoned`; [`render_frame`] guarantees it
    /// on every exit path.
    fn abandoned(&self, pass: u64);
    /// Whether `finished` needs each submission's GPU time ([`GpuSpan`]
    /// timestamps); without it the renderer reports wall time only.
    ///
    /// [`GpuSpan`]: crate::gpu_time::GpuSpan
    fn measures(&self) -> bool {
        true
    }
}

thread_local! {
    /// This thread's timestamp span for the device it renders on (each job
    /// engine lane renders on its own thread; spans are never shared).
    static SPAN: std::cell::RefCell<Option<(wgpu::Device, Arc<crate::gpu_time::GpuSpan>)>> =
        const { std::cell::RefCell::new(None) };
}

/// The thread's [`GpuSpan`](crate::gpu_time::GpuSpan) on `renderer`'s
/// device, made once per device; `None` without timestamp queries.
fn gpu_span(renderer: &Renderer) -> Option<Arc<crate::gpu_time::GpuSpan>> {
    SPAN.with(|cell| {
        let mut cell = cell.borrow_mut();
        if let Some((device, span)) = cell.as_ref()
            && device == renderer.device()
        {
            return Some(span.clone());
        }
        let span = Arc::new(crate::gpu_time::GpuSpan::new(
            renderer.device(),
            renderer.queue(),
        )?);
        *cell = Some((renderer.device().clone(), span.clone()));
        Some(span)
    })
}

/// One admitted submission: released as abandoned when dropped before
/// [`Admitted::finish`] (an early return, a panic).
struct Admitted<'a> {
    gate: &'a dyn GpuGate,
    pass: u64,
    open: bool,
}

impl Admitted<'_> {
    fn finish(mut self, units: u32, measured_us: f64, gpu_timed: bool) {
        self.open = false;
        self.gate
            .finished_timed(self.pass, units, measured_us, gpu_timed);
    }
}

impl Drop for Admitted<'_> {
    fn drop(&mut self) {
        if self.open {
            self.gate.abandoned(self.pass);
        }
    }
}

/// No interactive engine to protect (the CLI, tests): whole frames.
#[derive(Debug, Default, Clone, Copy)]
pub struct Unrestricted;

impl GpuGate for Unrestricted {
    fn admit(&self, _: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        (!cancel.load(Ordering::Relaxed)).then_some(units)
    }

    fn finished(&self, _: u64, _: u32, _: f64) {}

    fn abandoned(&self, _: u64) {}

    fn measures(&self) -> bool {
        false
    }
}

/// A slicing pass key for one job at one output size: units of the same job
/// and size cost the same.
pub fn pass_key(seed: u64, size: Size) -> u64 {
    let mut h = seed ^ 0x9e37_79b9_7f4a_7c15;
    for v in [u64::from(size.width), u64::from(size.height)] {
        h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
        h ^= h >> 29;
    }
    h
}

/// A rendered frame read back to the CPU.
pub struct ReadFrame {
    /// Premultiplied working values, top-down, `width × height × 4`.
    pub rgba: Vec<f32>,
    pub width: u32,
    pub height: u32,
    /// Layers that failed and drew nothing (section 8 isolation).
    pub failures: Vec<pm_render::LayerFailure>,
}

/// Waits for the frame's commands, the watchdog looking on
/// (`Renderer::wait_idle`).
fn wait_idle(renderer: &mut Renderer) {
    renderer.wait_idle();
}

/// Renders one frame through `gate`: the whole frame at once when the gate
/// admits every unit, else bands of rows as regions of interest. A one-off
/// frame (an agent still, a parity frame): the renderer keeps no staging
/// buffer or frame target afterwards (an export keeps them from frame to
/// frame, [`run_export_with_blender`]).
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    renderer: &mut Renderer,
    document: &Document,
    composition: &str,
    time: Ticks,
    request: &RenderRequest,
    media: &MediaPaths,
    gate: &dyn GpuGate,
    pass: u64,
    cancel: &AtomicBool,
) -> Result<ReadFrame, JobError> {
    let result = render_frame_reusing(
        renderer,
        document,
        composition,
        time,
        request,
        media,
        gate,
        pass,
        cancel,
    );
    renderer.release_frame_reuse();
    result
}

/// [`render_frame`], keeping the frame's staging buffer and target for the
/// next frame (an export's frames; the caller releases them with
/// `Renderer::release_frame_reuse` when its frames end, as
/// [`run_export_with_blender`] does).
#[allow(clippy::too_many_arguments)]
pub fn render_frame_reusing(
    renderer: &mut Renderer,
    document: &Document,
    composition: &str,
    time: Ticks,
    request: &RenderRequest,
    media: &MediaPaths,
    gate: &dyn GpuGate,
    pass: u64,
    cancel: &AtomicBool,
) -> Result<ReadFrame, JobError> {
    let span = if gate.measures() {
        gpu_span(renderer)
    } else {
        None
    };
    renderer.set_bracket(
        span.clone()
            .map(|s| s as Arc<dyn pm_render::CommandBracket>),
    );
    let result = render_bands(
        renderer,
        document,
        composition,
        time,
        request,
        media,
        gate,
        pass,
        cancel,
        span.as_deref(),
    );
    renderer.set_bracket(None);
    result
}

/// [`render_frame_reusing`]'s bands, with the renderer's bracket set to `span`.
#[allow(clippy::too_many_arguments)]
fn render_bands(
    renderer: &mut Renderer,
    document: &Document,
    composition: &str,
    time: Ticks,
    request: &RenderRequest,
    media: &MediaPaths,
    gate: &dyn GpuGate,
    pass: u64,
    cancel: &AtomicBool,
    span: Option<&crate::gpu_time::GpuSpan>,
) -> Result<ReadFrame, JobError> {
    let size = request.output_size;
    let (w, h) = (size.width, size.height);
    let total_units = h.div_ceil(BAND_ROWS);
    // Allocated by the first band unless it is the whole frame.
    let mut rgba = Vec::new();
    let mut failures = Vec::new();
    let mut y = 0u32;
    while y < h {
        if cancel.load(Ordering::Relaxed) {
            return Err(JobError::Cancelled);
        }
        let remaining = (h - y).div_ceil(BAND_ROWS);
        let admitted = gate
            .admit(pass, remaining, cancel)
            .ok_or(JobError::Cancelled)?
            .clamp(1, remaining);
        let admission = Admitted {
            gate,
            pass,
            open: true,
        };
        let band = (admitted * BAND_ROWS).min(h - y);
        let whole = y == 0 && band == h;
        let mut req = request.clone();
        if !whole {
            req.region = Some(PixelRect {
                x: 0,
                y,
                width: w,
                height: band,
            });
        }
        let mut frame = match renderer.render(document, composition, time, &req, media) {
            Ok(frame) => frame,
            Err(e) => {
                // Whatever the failed render submitted finishes before the
                // admission is released.
                wait_idle(renderer);
                drop(admission);
                return Err(e.into());
            }
        };
        let submitted = Instant::now();
        wait_idle(renderer);
        // A program ran away in this frame: its pixels are not delivered.
        if let Some(why) = renderer.watchdog_stopped() {
            drop(admission);
            return Err(pm_render::RenderError::Watchdog(why.to_owned()).into());
        }
        let wall_us = submitted.elapsed().as_secs_f64() * 1e6;
        // The admitted command buffer's GPU time when timestamps are
        // available (the wall time also counts the wait behind other GPU
        // clients); the readback below is its own small copy.
        let gpu_us = span
            .as_ref()
            .and_then(|s| s.read_us(renderer.device(), renderer.queue()));
        let units = if whole { total_units } else { admitted };
        admission.finish(units, gpu_us.unwrap_or(wall_us), gpu_us.is_some());
        let pixels = renderer.read_rgba_reusing(&frame)?;
        let row = w as usize * 4;
        let at = y as usize * row;
        if whole {
            // One band is the whole frame: its pixels are the frame.
            rgba = pixels;
        } else {
            if rgba.is_empty() {
                rgba = vec![0f32; w as usize * h as usize * 4];
            }
            rgba[at..at + pixels.len()].copy_from_slice(&pixels);
        }
        failures.append(&mut frame.failures);
        renderer.recycle(frame);
        y += band;
    }
    Ok(ReadFrame {
        rgba,
        width: w,
        height: h,
        failures,
    })
}

// ---------------------------------------------------------------------------
// Export settings → encoder spec
// ---------------------------------------------------------------------------

/// Frame and sample timing of an export range at the export rate (section 5,
/// rule 6: the export rate may differ from the composition's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    pub start: Ticks,
    pub rate: Rate,
    pub frames: u64,
}

impl Timeline {
    /// `max(1, round(duration × fps))` frames from `range.start`, as today's
    /// exporter counts them (`Math.round`, half up).
    pub fn new(start: Ticks, end: Ticks, rate: Rate) -> Result<Timeline, JobError> {
        let span = i128::from(end.get()) - i128::from(start.get());
        if span <= 0 {
            return Err(JobError::Invalid("the export range is empty".into()));
        }
        let (num, den) = (i128::from(rate.num()), i128::from(rate.den()));
        let unit = den * i128::from(TICKS_PER_SECOND);
        let frames = ((2 * span * num + unit) / (2 * unit)).max(1);
        let timeline = Timeline {
            start,
            rate,
            frames: u64::try_from(frames)
                .map_err(|_| JobError::Invalid("too many frames".into()))?,
        };
        // Every frame's time and the end of the last one (its audio) must be
        // representable ticks.
        let end_tick = i128::from(start.get()) + timeline.offset(timeline.frames);
        if i64::try_from(end_tick).map_or(true, |v| Ticks::new(v).is_err()) {
            return Err(JobError::Invalid(
                "the export range ends past the latest representable time".into(),
            ));
        }
        Ok(timeline)
    }

    /// Ticks from `start` to frame `n`: `n / rate`, rounded half to even
    /// onto the tick grid (exact for rates that divide the flick).
    fn offset(&self, n: u64) -> i128 {
        let (num, den) = (i128::from(self.rate.num()), i128::from(self.rate.den()));
        let x = i128::from(n) * den * i128::from(TICKS_PER_SECOND);
        let (q, r) = (x.div_euclid(num), x.rem_euclid(num));
        let up = match (2 * r).cmp(&num) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => q % 2 != 0,
            std::cmp::Ordering::Less => false,
        };
        q + i128::from(up)
    }

    /// The composition time of export frame `n` (`0 ..= frames`; `new`
    /// checked that these are representable): `start + n / rate`. Past
    /// `frames` it saturates at the latest representable time.
    pub fn tick(&self, n: u64) -> Ticks {
        Ticks::saturating_from_i128(i128::from(self.start.get()) + self.offset(n))
    }

    /// Project samples `[first, end)` of export frame `n` at `rate` Hz.
    pub fn samples(&self, n: u64, sample_rate: u32) -> (i64, i64) {
        let a = pm_audio::offline::sample_of_ticks(self.tick(n).get(), sample_rate);
        let b = pm_audio::offline::sample_of_ticks(self.tick(n + 1).get(), sample_rate);
        (a, b.max(a))
    }
}

/// The `tmcd` track of a MOV export (section 17, "Muxers"): it labels the
/// export's first frame with the composition's own timecode at the export
/// rate (frame `start × rate`, rounded half up, so 00:00:00:00 at the
/// composition start), drop-frame at 29.97 and 59.94 as the timeline shows
/// it, wrapped to one day. Start timecode and reel settings arrive with
/// Phase 3b presets (contract 12, open question 4).
pub fn start_timecode(timeline: &Timeline) -> crate::settings::TimecodeSpec {
    let format = pm_time::TimecodeFormat::for_rate(timeline.rate);
    let (num, den) = (
        i128::from(timeline.rate.num()),
        i128::from(timeline.rate.den()),
    );
    let unit = den * i128::from(TICKS_PER_SECOND);
    let first = (2 * i128::from(timeline.start.get()) * num + unit).div_euclid(2 * unit);
    let day = i128::from(format.frames_per_day());
    crate::settings::TimecodeSpec {
        start_frame: first.rem_euclid(day) as i64,
        format,
        reel: None,
    }
}

fn rate_of(settings: &ExportSettings) -> Result<Rate, JobError> {
    let fr = settings.frame_rate;
    Rate::new(i64::from(fr.num), i64::from(fr.den))
        .map_err(|e| JobError::Invalid(format!("frame rate {}/{}: {e}", fr.num, fr.den)))
}

/// The encoder and container for `settings` (section 17, "Formats at cutover").
pub fn codec_of(settings: &ExportSettings) -> Result<(Codec, Container), JobError> {
    let bitrate_bps = settings.quality.bitrate_mbps() * 1_000_000;
    Ok(match settings.format {
        ExportFormat::ProRes { profile } => (
            Codec::ProRes(match profile {
                WireProRes::Proxy => ProResProfile::Proxy,
                WireProRes::Lt => ProResProfile::Lt,
                WireProRes::Standard => ProResProfile::Standard,
                WireProRes::Hq => ProResProfile::Hq,
                WireProRes::P4444 => ProResProfile::P4444,
                WireProRes::P4444Xq => ProResProfile::P4444Xq,
            }),
            Container::Mov,
        ),
        ExportFormat::H264 => (Codec::H264 { bitrate_bps }, Container::Mp4),
        ExportFormat::Hevc { ten_bit, container } => (
            Codec::Hevc {
                bitrate_bps,
                ten_bit,
            },
            match container {
                WireContainer::Mp4 => Container::Mp4,
                WireContainer::Mov => Container::Mov,
            },
        ),
        ExportFormat::Webm { codec } => (
            match codec {
                WebmCodec::Vp9 => Codec::Vp9 { bitrate_bps },
                WebmCodec::Vp8 => Codec::Vp8 { bitrate_bps },
            },
            Container::Webm,
        ),
        ExportFormat::PngSequence { sixteen_bit } => {
            (Codec::Png { sixteen_bit }, Container::PngSequence)
        }
        ExportFormat::PngStill { sixteen_bit } => (Codec::Png { sixteen_bit }, Container::PngStill),
    })
}

fn tags_of(color: OutputColor) -> Result<ColorTags, JobError> {
    match color {
        OutputColor::Rec709Broadcast => Ok(ColorTags::Rec709Broadcast),
        OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        } => Ok(ColorTags::Rec709QuickTime),
        OutputColor::SrgbImage => Ok(ColorTags::SrgbImage),
        OutputColor::Rec709Image => Ok(ColorTags::Rec709Image),
        other => Err(JobError::Invalid(format!(
            "output color {other:?} is not an export tag at cutover"
        ))),
    }
}

/// The audio coding of an export: the delivery option, or today's audio for
/// the container (24-bit PCM in MOV, AAC in MP4, Opus in WebM).
pub fn audio_codec_of(settings: &ExportSettings, container: Container) -> AudioCodec {
    match (container, settings.delivery.audio_coding) {
        (Container::Webm, _) => AudioCodec::Opus,
        (_, Some(AudioCoding::Pcm16)) => AudioCodec::Pcm16,
        (_, Some(AudioCoding::Pcm24)) => AudioCodec::Pcm24,
        (_, Some(AudioCoding::PcmFloat)) => AudioCodec::PcmF32,
        (_, Some(AudioCoding::Aac)) => AudioCodec::Aac,
        (Container::Mov, None) => AudioCodec::Pcm24,
        _ => AudioCodec::Aac,
    }
}

/// The `tmcd` track of a MOV export: the start timecode and reel the user
/// chose (section 17, "Comfort"), or the composition's own timecode at the
/// first frame ([`start_timecode`]).
pub fn timecode_of(
    settings: &ExportSettings,
    timeline: &Timeline,
) -> Result<crate::settings::TimecodeSpec, JobError> {
    let Some(tc) = &settings.delivery.timecode else {
        return Ok(start_timecode(timeline));
    };
    let format = if tc.drop_frame {
        pm_time::TimecodeFormat::drop_frame(timeline.rate)
            .map_err(|_| JobError::Invalid("drop-frame timecode needs 29.97 or 59.94 fps".into()))?
    } else {
        pm_time::TimecodeFormat::non_drop(timeline.rate)
    };
    let start_frame = format
        .parse(&tc.start)
        .map_err(|e| JobError::Invalid(format!("start timecode {:?}: {e}", tc.start)))?;
    Ok(crate::settings::TimecodeSpec {
        start_frame: start_frame.rem_euclid(format.frames_per_day()),
        format,
        reel: tc.reel.clone().filter(|r| !r.is_empty()),
    })
}

/// One export as the job engine runs it.
#[derive(Clone)]
pub struct ExportJob {
    pub document: Arc<Document>,
    pub composition: String,
    pub settings: ExportSettings,
    pub media: Arc<MediaPaths>,
    /// The output: a path the user chose, or the host's temporary file
    /// (`Overwrite::InPlace`).
    pub destination: PathBuf,
    pub overwrite: Overwrite,
    /// Unix permission bits of the delivered file (agent exports: 0o600).
    pub mode: Option<u32>,
    /// Seeds the slicing pass key.
    pub seed: u64,
}

/// What an export will produce, checked before anything is written.
pub struct ExportPlan {
    pub spec: ExportSpec,
    pub timeline: Timeline,
    pub request: RenderRequest,
    /// The audio graph, when the export has audio and the composition has
    /// audible layers (today's exporter: `hasAudibleLayers`).
    pub audio: Option<AudioPlan>,
    /// The document rendered: the job's, or a copy without captions when they
    /// are delivered as a file instead of burned in.
    pub document: Arc<Document>,
    /// PNG sequences: the name frame files start with.
    pub sequence_name: Option<String>,
    /// Encoded-frame previews for the progress sheet.
    pub preview: bool,
    /// Runs over each rendered frame before it is encoded (an extension's
    /// compatibility context drawing over today's WebM export frames;
    /// pm-node `jobs::compat`). `None`: frames are encoded as rendered.
    pub frame_post: Option<Arc<dyn FramePost>>,
}

/// A pass over each rendered export frame before encoding ([`ExportPlan::frame_post`]).
pub trait FramePost: Send + Sync {
    /// Frame `index` at `time`: its premultiplied working values
    /// (`width × height × 4`, top-down, `working` encoding), replaced or
    /// returned unchanged. An error fails the export.
    fn frame(
        &self,
        index: u64,
        time: Ticks,
        width: u32,
        height: u32,
        working: crate::settings::WorkingEncoding,
        rgba: Vec<f32>,
    ) -> Result<Vec<f32>, JobError>;
}

pub struct AudioPlan {
    pub graph: Arc<pm_audio::MixGraph>,
    pub cache: Arc<pm_audio::SourceCache>,
    pub sample_rate: u32,
    /// The export's samples: today's `OfflineAudioContext` length
    /// ([`legacy_audio_samples`]), not the frames' span.
    pub samples: i64,
    /// The range in seconds as today's `renderOffline(t0, t1)` receives it:
    /// its clips and envelopes start from `t0` (`pm_audio::today`).
    pub range: (f64, f64),
}

/// Today's export mix length (`audio.ts` `renderOffline`):
/// `ceil((t1 − t0) × 48000)` with the range in seconds as JavaScript holds
/// them. Range ends are ticks of seconds the sheet chose, so the division
/// gives back the same double.
pub fn legacy_audio_samples(start: Ticks, end: Ticks, sample_rate: u32) -> i64 {
    let seconds = |t: Ticks| t.get() as f64 / TICKS_PER_SECOND as f64;
    let span = (seconds(end) - seconds(start)).max(0.0);
    (span * f64::from(sample_rate)).ceil() as i64
}

/// Source blocks kept decoded for one export's mix.
const AUDIO_CACHE_BYTES: usize = 256 << 20;

fn evaluator() -> pm_eval::Evaluator {
    pm_eval::Evaluator::new().with_expressions(Arc::new(pm_render::expr::PmExprEngine))
}

// ---------------------------------------------------------------------------
// Captions and offline media
// ---------------------------------------------------------------------------

/// `document` with every captions layer drawing nothing (its type becomes
/// `null`, keeping ids, order, and parenting), as today's exporter skips
/// captions when they are delivered as a caption file (`PM.captionsHidden`).
/// `None` when the document has no captions layers.
pub fn without_captions(document: &Document) -> Option<Document> {
    use pm_model::json::Value;
    fn hide(layers: &mut Value) -> bool {
        let Value::Array(items) = layers else {
            return false;
        };
        let mut changed = false;
        for layer in items {
            if let Value::Object(map) = layer {
                let captions = map.get("type").is_some_and(
                    |t| matches!(t, Value::String(s) if s.to_string_lossy() == "captions"),
                );
                if captions {
                    map.insert("type", Value::from("null"));
                    changed = true;
                }
            }
        }
        changed
    }
    let mut root = document.to_value()?;
    let Value::Object(map) = &mut root else {
        return None;
    };
    let mut changed = map.get_mut("layers").is_some_and(hide);
    if let Some(Value::Object(comps)) = map.get_mut("comps") {
        for (_, comp) in comps.iter_mut() {
            if let Value::Object(c) = comp
                && let Some(layers) = c.get_mut("layers")
            {
                changed |= hide(layers);
            }
        }
    }
    changed.then(|| Document::new(Some(root), document.revision()))
}

/// A clip the export needs whose file the host could not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineClip {
    pub asset: String,
    /// The asset's name, for messages ("2 clips are offline").
    pub name: String,
}

/// The media of `composition` (and every composition it nests) that is
/// offline: video, image, and audio layers whose asset record exists but has
/// no file in `media`. Assets without a record draw nothing, as today, and
/// are not listed.
pub fn offline_media(
    document: &Document,
    composition: &str,
    media: &MediaPaths,
) -> Vec<OfflineClip> {
    use pm_eval::node;
    let Some(root) = document.root() else {
        return vec![];
    };
    let open = node::string(node::member(Some(root), "compId"))
        .or_else(|| node::string(node::member(Some(root), "id")))
        .map(|s| s.to_string_lossy());
    let comp_node = |id: &str| -> Option<&Node> {
        if open.as_deref() == Some(id) && node::member(Some(root), "layers").is_some() {
            return Some(root);
        }
        node::member(node::member(Some(root), "comps"), id)
    };
    let assets = node::member(Some(root), "assets");
    let string = |n: Option<&Node>| node::js_value_opt(n).to_js_string().to_string_lossy();
    let mut seen = std::collections::BTreeSet::new();
    let mut out: Vec<OfflineClip> = vec![];
    let mut stack = vec![composition.to_owned()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(comp) = comp_node(&id) else {
            continue;
        };
        for layer in node::array(node::member(Some(comp), "layers")).unwrap_or(&[]) {
            let d = node::member(Some(layer), "d");
            match pm_render::js::layer_type(layer).as_str() {
                "precomp" => {
                    if let Some(c) = node::member(d, "comp").filter(|c| node::truthy(Some(c))) {
                        let c = string(Some(c));
                        if open.as_deref() != Some(c.as_str())
                            || node::member(Some(root), "layers").is_none()
                        {
                            stack.push(c);
                        }
                    }
                }
                "video" | "image" | "audio" => {
                    let Some(asset) = node::member(d, "asset").map(|a| string(Some(a))) else {
                        continue;
                    };
                    let Some(record) = node::member(assets, &asset) else {
                        continue;
                    };
                    if !media.paths.contains_key(&asset) && !out.iter().any(|c| c.asset == asset) {
                        let name = node::member(Some(record), "name")
                            .map(|n| string(Some(n)))
                            .filter(|n| !n.is_empty())
                            .unwrap_or_else(|| asset.clone());
                        out.push(OfflineClip { asset, name });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Validates `job` and builds its plan: capability routing (pm-render's
/// `unsupported_with_media`, audio sources pm-audio cannot open), the encoder
/// spec, and the audio graph. Offline media is not checked here; the job
/// engine asks [`offline_media`] first (queued jobs wait for it, section 14.8).
pub fn plan_export(job: &ExportJob) -> Result<ExportPlan, JobError> {
    plan_export_with(job, false)
}

/// [`plan_export`] for a job engine with a Blender stage: compositions
/// rendered with Blender are not routed for that reason
/// (`crate::blender`).
pub fn plan_export_with(job: &ExportJob, blender: bool) -> Result<ExportPlan, JobError> {
    plan_export_checked(job, blender, &|doc, composition, media| {
        pm_render::unsupported_with_media(doc, composition, media)
    })
}

/// [`plan_export_with`] routed with `renderer`'s own capabilities
/// (`Renderer::unsupported_with_media`): the definitions the job embeds
/// (contract 10; set on the renderer before planning) draw its effects,
/// transitions, and extension layers, and its fonts its text.
pub fn plan_export_for(
    job: &ExportJob,
    blender: bool,
    renderer: &Renderer,
) -> Result<ExportPlan, JobError> {
    plan_export_checked(job, blender, &|doc, composition, media| {
        renderer.unsupported_with_media(doc, composition, media)
    })
}

/// The capability check an export plan routes with.
pub type CapabilityCheck<'a> =
    dyn Fn(&Document, &str, &MediaPaths) -> Vec<pm_render::Unsupported> + 'a;

fn plan_export_checked(
    job: &ExportJob,
    blender: bool,
    check: &CapabilityCheck<'_>,
) -> Result<ExportPlan, JobError> {
    let s = &job.settings;
    pm_protocol::validate::Validate::validate(s).map_err(|v| JobError::Invalid(v.to_string()))?;
    let document = if s.delivery.hide_captions {
        without_captions(&job.document).map_or_else(|| job.document.clone(), Arc::new)
    } else {
        job.document.clone()
    };
    let doc = &*document;
    let (codec, container) = codec_of(s)?;
    let tags = tags_of(s.color)?;
    let mut problems: Vec<String> = check(doc, &job.composition, &job.media)
        .iter()
        .filter(|u| !(blender && u.feature == pm_render::Feature::Blender))
        .map(ToString::to_string)
        .collect();
    let rate = rate_of(s)?;
    let mut timeline = Timeline::new(s.range.start, s.range.end, rate)?;
    if container == Container::PngStill {
        // A still is the range's first frame.
        timeline.frames = 1;
    }
    let alpha = match s.alpha {
        WireAlpha::None => AlphaOutput::None,
        WireAlpha::Straight => AlphaOutput::Straight,
        WireAlpha::Premultiplied => AlphaOutput::Premultiplied,
    };
    let audio_codec = audio_codec_of(s, container);
    let audio = if s.audio && !s.format.is_image() {
        let cache = pm_audio::SourceCache::new(AUDIO_CACHE_BYTES, 2);
        // An export takes today's exact lengths before it mixes.
        cache.set_blocking_lengths(true);
        let graph =
            pm_audio::MixGraph::build(doc, &job.composition, &mut evaluator(), &cache, &*job.media)
                .map_err(|e| JobError::Audio(e.to_string()))?;
        for (name, asset, why) in &graph.problems {
            problems.push(format!("audio of {name} ({asset}): {why}"));
        }
        if graph.voices.is_empty() {
            None
        } else {
            Some(AudioPlan {
                // Today's export OfflineAudioContext is always stereo 48 kHz,
                // independently of the project's playback device/rate.
                sample_rate: 48_000,
                samples: legacy_audio_samples(s.range.start, s.range.end, 48_000),
                range: (
                    s.range.start.get() as f64 / TICKS_PER_SECOND as f64,
                    s.range.end.get() as f64 / TICKS_PER_SECOND as f64,
                ),
                graph: Arc::new(graph),
                cache,
            })
        }
    } else {
        None
    };
    if !problems.is_empty() {
        return Err(JobError::Unsupported(problems));
    }
    let spec = ExportSpec {
        width: s.size.width,
        height: s.size.height,
        rate,
        frames: timeline.frames,
        codec,
        container,
        tags,
        alpha,
        working: WorkingEncoding::Standard,
        dither: if s.dither { Dither::On } else { Dither::Off },
        audio: audio.as_ref().map(|a| AudioSpec {
            sample_rate: a.sample_rate,
            channels: 2,
            codec: audio_codec,
        }),
        timecode: if container == Container::Mov {
            Some(timecode_of(s, &timeline)?)
        } else {
            None
        },
        destination: job.destination.clone(),
        overwrite: job.overwrite,
        mode: job.mode,
    };
    spec.validate()?;
    let request = RenderRequest {
        output_size: s.size,
        scale: None,
        region: None,
        preview_canvas: false,
        precision: s.precision,
        depth_id: false,
        // Today's alpha exports render without the composition background.
        background: alpha == AlphaOutput::None,
        export: true,
        view: None,
        blender: Default::default(),
        // Today's export counts: 16 samples opaque, 20 with alpha (the
        // composition's `motionBlurSamples` when set).
        motion_blur: if s.motion_blur {
            pm_render::motion::MotionBlur::Kind(if alpha == AlphaOutput::None {
                pm_protocol::frame::MotionBlurKind::Export
            } else {
                pm_protocol::frame::MotionBlurKind::AlphaExport
            })
        } else {
            pm_render::motion::MotionBlur::Off
        },
        sampling: pm_render::sampling::Sampling::Final,
        // `prepareFrame`'s end padding: the last frame of a clip is kept.
        video_end: VideoEnd::Final,
        origin: None,
    };
    Ok(ExportPlan {
        spec,
        timeline,
        request,
        audio,
        document,
        sequence_name: s.delivery.sequence_name.clone(),
        preview: s.delivery.preview,
        frame_post: None,
    })
}

/// A small JPEG of an encoded frame for the export progress sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewFrame {
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

/// Previews at most this often.
pub const PREVIEW_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// A box-filtered preview of a premultiplied working frame, at most
/// `max_edge` pixels on its long edge, over black, as JPEG.
pub fn preview_jpeg(
    rgba: &[f32],
    width: u32,
    height: u32,
    max_edge: u32,
) -> Result<(u32, u32, Vec<u8>), JobError> {
    let scale = (f64::from(max_edge) / f64::from(width.max(height))).min(1.0);
    let (pw, ph) = (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    );
    let mut out = vec![0u8; pw as usize * ph as usize * 4];
    let (w, h) = (width as usize, height as usize);
    for py in 0..ph as usize {
        let y0 = py * h / ph as usize;
        let y1 = ((py + 1) * h / ph as usize).max(y0 + 1);
        for px in 0..pw as usize {
            let x0 = px * w / pw as usize;
            let x1 = ((px + 1) * w / pw as usize).max(x0 + 1);
            let mut sum = [0f32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 4;
                    for k in 0..3 {
                        sum[k] += rgba[i + k];
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            let o = (py * pw as usize + px) * 4;
            for k in 0..3 {
                let v = sum[k] / n;
                out[o + k] =
                    (if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) } * 255.0).round() as u8;
            }
            out[o + 3] = 255;
        }
    }
    Ok((pw, ph, encode_jpeg(&out, pw, ph, 0.8)?))
}

/// Progress of a running export.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub frames_done: u64,
    pub frames: u64,
    /// A preview of the frame just encoded (plans that ask for previews, at
    /// most every [`PREVIEW_EVERY`], and the last frame).
    pub preview: Option<Arc<PreviewFrame>>,
}

impl Progress {
    pub fn fraction(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            (self.frames_done as f64 / self.frames as f64).clamp(0.0, 1.0)
        }
    }
}

enum Rendered {
    Frame(u64, Vec<f32>),
    Failed(JobError),
}

/// Runs a planned export into `sink`: renders on this thread, converts,
/// mixes, and encodes on a second one. `progress` sees each encoded frame.
#[allow(clippy::too_many_arguments)]
pub fn run_export_into<S: Sink + Send>(
    renderer: &mut Renderer,
    job: &ExportJob,
    plan: ExportPlan,
    sink: S,
    gate: &dyn GpuGate,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<S::Output, JobError>
where
    S::Output: Send,
{
    run_export_with_blender(renderer, job, plan, sink, gate, cancel, progress, None)
}

/// [`run_export_into`] with the job lane's Blender stage: each frame's
/// Blender surfaces are prepared before it renders (`crate::blender`).
#[allow(clippy::too_many_arguments)]
pub fn run_export_with_blender<S: Sink + Send>(
    renderer: &mut Renderer,
    job: &ExportJob,
    plan: ExportPlan,
    sink: S,
    gate: &dyn GpuGate,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
    blender: Option<&mut crate::blender::ExportBlender>,
) -> Result<S::Output, JobError>
where
    S::Output: Send,
{
    let result = export_frames(renderer, job, plan, sink, gate, cancel, progress, blender);
    // The frames' staging buffer, target and pixels are kept only while
    // exporting (one-off frames release them in `render_frame`).
    renderer.release_frame_reuse();
    result
}

#[allow(clippy::too_many_arguments)]
fn export_frames<S: Sink + Send>(
    renderer: &mut Renderer,
    job: &ExportJob,
    plan: ExportPlan,
    sink: S,
    gate: &dyn GpuGate,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
    mut blender: Option<&mut crate::blender::ExportBlender>,
) -> Result<S::Output, JobError>
where
    S::Output: Send,
{
    let ExportPlan {
        spec,
        timeline,
        request,
        audio,
        document,
        preview: wants_preview,
        frame_post,
        ..
    } = plan;
    let working = spec.working;
    let frames = spec.frames;
    let pass = pass_key(job.seed, request.output_size);
    let mut pipeline = Pipeline::new(spec, sink)?;
    pipeline.set_legacy_png(crate::png_compat::has_level1_content(&document));
    let mut mixer = audio.map(|a| {
        let first = pm_audio::offline::sample_of_ticks(timeline.tick(0).get(), a.sample_rate);
        let mut mixer = pm_audio::Mixer::new(
            pm_audio::MixFormat::stereo(a.sample_rate),
            document.clone(),
            a.graph,
            evaluator(),
            a.cache,
        );
        // Today's OfflineAudioContext starts at the range's start.
        mixer.set_today_range(a.range.0.max(0.0), a.range.1, first);
        mixer.set_today_playhead(
            job.settings
                .audio_index_playhead
                .map(|t| t.get() as f64 / TICKS_PER_SECOND as f64),
        );
        (
            mixer,
            a.sample_rate,
            // Today's mix ends here, before or after the last frame's end.
            first + a.samples,
        )
    });
    let (tx, rx) = std::sync::mpsc::sync_channel::<Rendered>(1);
    // The encoder gives each frame's pixels back for a later frame to read
    // into (docs/engine/perfbench.md).
    let (back_tx, back_rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let qos = crate::par::current_qos();
    std::thread::scope(|scope| {
        let encoder = scope.spawn(move || -> Result<S::Output, JobError> {
            crate::par::apply_qos(qos);
            let mut last_preview: Option<Instant> = None;
            for expected in 0..frames {
                let (n, rgba) = match rx.recv() {
                    Ok(Rendered::Frame(n, rgba)) => (n, rgba),
                    Ok(Rendered::Failed(e)) => return Err(e),
                    Err(_) => return Err(JobError::Cancelled),
                };
                debug_assert_eq!(n, expected);
                let (w, h) = (request.output_size.width, request.output_size.height);
                pipeline.push_frame(&WorkingFrame {
                    width: w,
                    height: h,
                    rgba: &rgba,
                })?;
                let due = wants_preview
                    && (n + 1 == frames
                        || last_preview.is_none_or(|t| t.elapsed() >= PREVIEW_EVERY));
                let preview = if due {
                    last_preview = Some(Instant::now());
                    let (pw, ph, jpeg) =
                        preview_jpeg(&rgba, w, h, pm_protocol::priority::JOB_PREVIEW_MAX_EDGE)?;
                    Some(Arc::new(PreviewFrame {
                        frame: n,
                        width: pw,
                        height: ph,
                        jpeg,
                    }))
                } else {
                    None
                };
                let _ = back_tx.send(rgba);
                if let Some((mixer, rate, last)) = mixer.as_mut() {
                    let (first, end) = timeline.samples(n, *rate);
                    let first = first.min(*last);
                    let end = if n + 1 == frames {
                        *last
                    } else {
                        end.min(*last)
                    };
                    let mut block = Vec::new();
                    let summary = pm_audio::offline::render(mixer, first, end, 8192, |_, inter| {
                        block.extend_from_slice(inter);
                        Ok(())
                    })
                    .map_err(|e| JobError::Audio(e.to_string()))?;
                    if let Some((voice, why)) = summary.errors.first() {
                        return Err(JobError::Audio(format!("{voice}: {why}")));
                    }
                    if !block.is_empty() {
                        pipeline.push_audio(&block)?;
                    }
                }
                progress(Progress {
                    frames_done: n + 1,
                    frames,
                    preview,
                });
            }
            pipeline
                .finish_with_cancel(cancel)
                .map_err(|error| match error {
                    crate::Error::Cancelled => JobError::Cancelled,
                    error => JobError::Export(error),
                })
        });
        let mut render_error = None;
        for n in 0..frames {
            if cancel.load(Ordering::Relaxed) {
                render_error = Some(JobError::Cancelled);
                break;
            }
            // Three frames' pixels circulate. Before frame n the encoder has
            // taken frame n - 2 (the channel holds one), so it has given
            // frame n - 3's back: waiting for it never stalls the export, and
            // which buffer a frame reads into does not depend on timing. An
            // encoder that stopped closes the channel: a new buffer then.
            if n >= PIXEL_BUFFERS
                && let Ok(rgba) = back_rx.recv()
            {
                renderer.give_rgba(rgba);
            }
            let frame_request = match blender.as_deref_mut() {
                None => Ok(request.clone()),
                Some(stage) => stage
                    .prepare(
                        renderer,
                        &document,
                        &job.composition,
                        timeline.tick(n),
                        &request,
                        &*job.media,
                        cancel,
                        &mut |_, _| {},
                    )
                    .map(|surfaces| crate::blender::with_surfaces(&request, surfaces)),
            };
            let rendered = frame_request
                .and_then(|frame_request| {
                    render_frame_reusing(
                        renderer,
                        &document,
                        &job.composition,
                        timeline.tick(n),
                        &frame_request,
                        &job.media,
                        gate,
                        pass,
                        cancel,
                    )
                })
                .and_then(|f| {
                    if f.failures.is_empty() {
                        Ok(f)
                    } else if let Some(l) = f
                        .failures
                        .iter()
                        .find(|l| l.message == pm_render::blender::NOT_PREPARED)
                    {
                        // A Blender surface is missing: the export fails
                        // (contract 11 invariant 7), it is never routed.
                        Err(JobError::Blender(pm_blender::BlenderError::Failed(
                            format!("layer {} in {}: {}", l.layer, l.composition, l.message),
                        )))
                    } else {
                        Err(JobError::Unsupported(
                            f.failures
                                .iter()
                                .map(|l| {
                                    format!("layer {} in {}: {}", l.layer, l.composition, l.message)
                                })
                                .collect(),
                        ))
                    }
                });
            // The frame as the exporting window's compatibility context
            // leaves it (today's WebM frames: `frame:rendered` handlers drew
            // on the compositor's canvas before it was encoded).
            let rendered = rendered.and_then(|f| match &frame_post {
                None => Ok(f.rgba),
                Some(post) => post.frame(
                    n,
                    timeline.tick(n),
                    request.output_size.width,
                    request.output_size.height,
                    working,
                    f.rgba,
                ),
            });
            match rendered {
                Ok(rgba) => {
                    if tx.send(Rendered::Frame(n, rgba)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    render_error = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = render_error {
            // The encoder side stops at the failure (or sees the channel
            // close); its partial output is removed when the sink drops.
            let _ = tx.send(Rendered::Failed(e));
        }
        drop(tx);
        encoder
            .join()
            .unwrap_or_else(|_| Err(JobError::Render("the encoder thread panicked".into())))
    })
}

/// Frames' pixel buffers an export keeps in circulation between the render
/// and encoder threads.
const PIXEL_BUFFERS: u64 = 3;

/// The sink for a planned export: VideoToolbox and AVAssetWriter (MOV, MP4),
/// FFmpeg (WebM), or the engine's PNG writer.
pub fn sink_for(plan: &ExportPlan) -> Result<crate::OutputSink, JobError> {
    sink_for_with(plan, None)
}

/// [`sink_for`] with the host's WebCodecs encoders, which the desktop app's
/// job engine has for the WebM exports today's exporter encodes with them.
pub fn sink_for_with(
    plan: &ExportPlan,
    encoder: Option<Box<dyn crate::webcodecs::EncodeHost>>,
) -> Result<crate::OutputSink, JobError> {
    let level1 = crate::png_compat::has_level1_content(&plan.document);
    let mut sink = crate::OutputSink::for_job_with(&plan.spec, level1, encoder)?;
    if let (crate::OutputSink::Png(png), Some(name)) = (&mut sink, &plan.sequence_name) {
        png.set_stem(name);
    }
    Ok(sink)
}

/// Plans and runs an export of any format, with verification and the atomic
/// move.
pub fn run_export(
    renderer: &mut Renderer,
    job: &ExportJob,
    gate: &dyn GpuGate,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<crate::Delivered, JobError> {
    let plan = plan_export(job)?;
    let sink = sink_for(&plan)?;
    run_export_into(renderer, job, plan, sink, gate, cancel, progress)
}

// ---------------------------------------------------------------------------
// Agent stills
// ---------------------------------------------------------------------------

/// The output size of an agent still: the composition scaled so its longest
/// edge is at most `max_dimension`, never enlarged (today's
/// `Export.snapshot`: `s = min(1, maxW / w)`), at least 2 × 2.
pub fn still_size(comp_w: u32, comp_h: u32, max_dimension: u32) -> Size {
    let (w, h) = (f64::from(comp_w.max(1)), f64::from(comp_h.max(1)));
    let s = (f64::from(max_dimension) / w.max(h)).min(1.0);
    Size::new(
        ((w * s).round() as u32).max(2),
        ((h * s).round() as u32).max(2),
    )
}

/// The composition's own size (`w`, `h`), as the legacy compositor reads it.
pub fn composition_size(document: &Document, composition: &str) -> Result<(u32, u32), JobError> {
    let comp = pm_eval::Comp::resolve(document, composition)
        .map_err(|_| JobError::Invalid(format!("composition {composition:?} not found")))?;
    let dim = |k: &str| {
        let v = pm_eval::node::to_number(comp.member(k));
        if v.is_finite() && v >= 1.0 {
            Ok(v.round() as u32)
        } else {
            Err(JobError::Invalid(format!(
                "composition {composition:?} has no {k}"
            )))
        }
    };
    Ok((dim("w")?, dim("h")?))
}

/// Premultiplied working values (Standard: E-encoded, over black when the
/// background is transparent) to 8-bit RGBA as a unorm render-target write
/// stores them (clamp, scale, round half to even; NaN is 0), which is what
/// today's canvas readback holds.
pub fn to_rgba8(rgba: &[f32]) -> Vec<u8> {
    rgba.iter()
        .map(|v| {
            let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            (v * 255.0).round_ties_even() as u8
        })
        .collect()
}

/// Encodes an opaque RGBA8 image as baseline JPEG (4:2:0) at `quality` in
/// (0, 1]; alpha is ignored (premultiplied values are already over black, as
/// a canvas's `toDataURL('image/jpeg')` composites them).
pub fn encode_jpeg(
    rgba8: &[u8],
    width: u32,
    height: u32,
    quality: f32,
) -> Result<Vec<u8>, JobError> {
    let rgb: Vec<u8> = rgba8
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let q = (quality.clamp(0.01, 1.0) * 100.0).round() as u8;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q.max(1))
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| JobError::Render(format!("JPEG: {e}")))?;
    Ok(out)
}

/// Encodes RGBA8 as PNG.
pub fn encode_png(rgba8: &[u8], width: u32, height: u32) -> Result<Vec<u8>, JobError> {
    let mut out = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut out),
        rgba8,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| JobError::Render(format!("PNG: {e}")))?;
    Ok(out)
}

/// Capability routing of agent stills (as [`plan_export`] routes exports):
/// the renderer's own check (`Renderer::unsupported_with_media`: its
/// definitions and fonts) for each composition, checked once per batch and
/// definition set because it opens media headers.
#[derive(Debug, Default)]
pub struct StillRouting {
    checked: std::collections::HashMap<(String, Option<u64>), Vec<String>>,
}

impl StillRouting {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Err(Unsupported)` when the engine cannot render `composition` with
    /// `renderer` as configured.
    pub fn check(
        &mut self,
        renderer: &Renderer,
        document: &Document,
        composition: &str,
        media: &MediaPaths,
    ) -> Result<(), JobError> {
        let generation = renderer
            .effects()
            .configured()
            .then(|| renderer.effects().generation().map_or(0, |g| g.get()));
        let problems = self
            .checked
            .entry((composition.to_owned(), generation))
            .or_insert_with(|| {
                renderer
                    .unsupported_with_media(document, composition, media)
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            });
        if problems.is_empty() {
            Ok(())
        } else {
            Err(JobError::Unsupported(problems.clone()))
        }
    }
}

/// One agent still: the composition at `time`, opaque (today's
/// `renderFrameTo` with the background), 16-bit intermediates, motion blur
/// off. Capability routing as for exports, through `routing`, before any
/// GPU work is admitted.
#[allow(clippy::too_many_arguments)]
pub fn render_still(
    renderer: &mut Renderer,
    document: &Document,
    composition: &str,
    time: Ticks,
    max_dimension: u32,
    media: &MediaPaths,
    routing: &mut StillRouting,
    gate: &dyn GpuGate,
    seed: u64,
    cancel: &AtomicBool,
) -> Result<ReadFrame, JobError> {
    routing.check(renderer, document, composition, media)?;
    let (cw, ch) = composition_size(document, composition)?;
    let size = still_size(cw, ch, max_dimension);
    // Today's agent stills run `prepareFrame` first (`snapshotAsync`).
    let request = RenderRequest {
        video_end: VideoEnd::Final,
        ..RenderRequest::sized(size, Precision::Float16)
    };
    let frame = render_frame(
        renderer,
        document,
        composition,
        time,
        &request,
        media,
        gate,
        pass_key(seed, size),
        cancel,
    )?;
    if !frame.failures.is_empty() {
        return Err(JobError::Unsupported(
            frame
                .failures
                .iter()
                .map(|l| format!("layer {} in {}: {}", l.layer, l.composition, l.message))
                .collect(),
        ));
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_counts_frames_like_todays_exporter_and_lands_on_the_grid() {
        let s = Ticks::from_whole_seconds(1);
        let e = Ticks::from_whole_seconds(3);
        let t = Timeline::new(s, e, Rate::FPS_23_976).unwrap();
        // round(2 × 23.976) = 48.
        assert_eq!(t.frames, 48);
        // Frame n lands on n × 1001/24000 s exactly (the rate divides the flick).
        for n in [0u64, 1, 47, 48] {
            let want = s.get() as i128 + n as i128 * 1001 * TICKS_PER_SECOND as i128 / 24000;
            assert_eq!(t.tick(n).get() as i128, want);
        }
        // A range whose end frame is past the latest tick is refused, not
        // wrapped back to the start.
        let late = Ticks::new(Ticks::MAX.get() - TICKS_PER_SECOND).unwrap();
        // 24 frames at 23.976 end 1.001 s after `late`: past the latest tick.
        assert!(Timeline::new(late, Ticks::MAX, Rate::FPS_23_976).is_err());
        let t = Timeline::new(late, Ticks::MAX, Rate::FPS_30).unwrap();
        assert!(t.tick(t.frames) <= Ticks::MAX && t.tick(t.frames) > t.tick(0));
        assert_eq!(t.tick(u64::MAX >> 8), Ticks::MAX);
        let one = Timeline::new(Ticks::ZERO, Ticks::new(1).unwrap(), Rate::FPS_30).unwrap();
        assert_eq!(one.frames, 1, "at least one frame");
        assert!(Timeline::new(e, s, Rate::FPS_30).is_err());
        // Half a frame rounds up (Math.round).
        let half = Ticks::new(TICKS_PER_SECOND / 60).unwrap();
        assert_eq!(
            Timeline::new(Ticks::ZERO, half, Rate::FPS_30)
                .unwrap()
                .frames,
            1
        );
        let span = Ticks::new(TICKS_PER_SECOND / 60 * 3).unwrap();
        assert_eq!(
            Timeline::new(Ticks::ZERO, span, Rate::FPS_30)
                .unwrap()
                .frames,
            2
        );
    }

    #[test]
    fn frame_samples_tile_the_range_exactly() {
        let t = Timeline::new(Ticks::ZERO, Ticks::from_whole_seconds(10), Rate::FPS_29_97).unwrap();
        let mut at = t.samples(0, 48_000).0;
        let mut total = 0;
        for n in 0..t.frames {
            let (a, b) = t.samples(n, 48_000);
            assert_eq!(a, at, "frame {n} starts where the last ended");
            total += b - a;
            at = b;
        }
        // round(frames / 29.97 × 48000).
        let want = (t.frames as f64 * 1001.0 / 30000.0 * 48000.0).round() as i64;
        assert_eq!(total, want);
    }

    #[test]
    fn still_sizes_follow_todays_snapshot() {
        assert_eq!(still_size(1920, 1080, 720), Size::new(720, 405));
        assert_eq!(still_size(1080, 1920, 720), Size::new(405, 720));
        assert_eq!(
            still_size(320, 180, 720),
            Size::new(320, 180),
            "never enlarged"
        );
        assert_eq!(still_size(4000, 2, 720), Size::new(720, 2), "at least 2");
    }

    #[test]
    fn pass_keys_differ_by_size_and_seed() {
        let a = pass_key(1, Size::new(1920, 1080));
        assert_ne!(a, pass_key(2, Size::new(1920, 1080)));
        assert_ne!(a, pass_key(1, Size::new(1080, 1920)));
        assert_eq!(a, pass_key(1, Size::new(1920, 1080)));
    }

    #[test]
    fn rgba8_rounds_and_clamps() {
        assert_eq!(to_rgba8(&[-0.1, 0.0, 0.5, 1.2]), vec![0, 0, 128, 255]);
        assert_eq!(to_rgba8(&[f32::NAN]), vec![0], "NaN is 0");
    }

    #[test]
    fn jpeg_and_png_encode() {
        let (w, h) = (16u32, 8u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [i as u8, 128, 255 - i as u8, 255])
            .collect();
        let jpeg = encode_jpeg(&rgba, w, h, 0.74).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        let png = encode_png(&rgba, w, h).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
