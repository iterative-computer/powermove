//! `pm-cli render` (plan section 22, "The CLI"): renders a project file (PMV4,
//! PMV3, or JSON) to a video through the same pipeline as the job engine:
//! capability check, pm-render frames, pm-audio's offline mix, VideoToolbox,
//! AVAssetWriter, verification, and an atomic move to the path the user
//! chose. Embedded media is unpacked to a private temporary folder; linked
//! media is read from its `sourcePath`; `--media <asset>=<file>` overrides
//! either.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pm_export::Overwrite;
use pm_export::render::{
    ExportJob, JobError, MediaPaths, Progress, Unrestricted, composition_size,
};
use pm_model::{Document, Value};
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput, ExportFormat, ExportSettings, ProResProfile, QualityTier,
};
use pm_render::Renderer;

use crate::corpus::{member, string};

/// Why a render did not produce a file: usage (exit 2), the engine routes it
/// (exit 3), or it failed (exit 1).
pub enum Failure {
    Usage(String),
    Unsupported(Vec<String>),
    Failed(String),
}

impl From<String> for Failure {
    fn from(s: String) -> Self {
        Failure::Failed(s)
    }
}

/// Presets (section 17, "Presets and the render queue"), the formats the job
/// engine encodes at Phase 2.
pub const PRESETS: &[(&str, &str)] = &[
    ("master", "Master · ProRes 422 HQ"),
    ("master-alpha", "Master with alpha · ProRes 4444"),
    ("web", "Web · H.264"),
];

#[derive(Debug, Default)]
pub struct Options {
    pub input: PathBuf,
    pub output: PathBuf,
    pub preset: Option<String>,
    pub format: Option<String>,
    pub quality: Option<String>,
    pub alpha: Option<String>,
    pub range: Option<String>,
    pub fps: Option<String>,
    pub scale: Option<f64>,
    pub size: Option<(u32, u32)>,
    pub no_audio: bool,
    pub color: Option<String>,
    pub dither: Option<bool>,
    pub precision: Option<u32>,
    pub composition: Option<String>,
    pub force: bool,
    pub media: Vec<(String, PathBuf)>,
}

fn usage(m: impl Into<String>) -> Failure {
    Failure::Usage(m.into())
}

fn format_of(name: &str) -> Result<ExportFormat, Failure> {
    let prores = |profile| Ok(ExportFormat::ProRes { profile });
    match name {
        "prores-proxy" => prores(ProResProfile::Proxy),
        "prores-lt" => prores(ProResProfile::Lt),
        "prores" | "prores-422" => prores(ProResProfile::Standard),
        "prores-hq" | "prores-422-hq" => prores(ProResProfile::Hq),
        "prores-4444" => prores(ProResProfile::P4444),
        "prores-4444xq" | "prores-4444-xq" => prores(ProResProfile::P4444Xq),
        "h264" => Ok(ExportFormat::H264),
        other => Err(usage(format!(
            "unknown format {other:?} (prores-proxy, prores-lt, prores, prores-hq, prores-4444, prores-4444xq, h264)"
        ))),
    }
}

fn rate_of(text: &str) -> Result<FrameRate, Failure> {
    let parse = |s: &str| s.trim().parse::<u32>().ok();
    if let Some((n, d)) = text.split_once('/') {
        return match (parse(n), parse(d)) {
            (Some(n), Some(d)) if n > 0 && d > 0 => Ok(FrameRate::new(n, d)),
            _ => Err(usage(format!("frame rate {text:?}"))),
        };
    }
    // Today's export list names NTSC rates by their decimals.
    Ok(match text {
        "23.976" => FrameRate::new(24000, 1001),
        "29.97" => FrameRate::new(30000, 1001),
        "47.952" => FrameRate::new(48000, 1001),
        "59.94" => FrameRate::new(60000, 1001),
        "119.88" => FrameRate::new(120000, 1001),
        "239.76" => FrameRate::new(240000, 1001),
        other => match parse(other) {
            Some(n) if n > 0 => FrameRate::new(n, 1),
            _ => return Err(usage(format!("frame rate {other:?}"))),
        },
    })
}

fn seconds_to_ticks(s: f64) -> Result<Ticks, Failure> {
    let t = pm_time::ticks_from_seconds(s).map_err(|e| usage(format!("time {s}: {e}")))?;
    Ticks::new(t).map_err(|_| usage(format!("time {s} is out of range")))
}

/// MIME type → file extension for unpacked media (pm-media sniffs content,
/// but FFmpeg and ImageIO read better with the right extension).
fn extension_for(mime: &str, record: Option<&Value>) -> String {
    let known = match mime {
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "video/x-matroska" => Some("mkv"),
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/heic" | "image/heif" => Some("heic"),
        "image/tiff" => Some("tiff"),
        "image/avif" => Some("avif"),
        "image/svg+xml" => Some("svg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/aac" => Some("aac"),
        "audio/flac" => Some("flac"),
        "audio/ogg" | "audio/opus" => Some("ogg"),
        _ => None,
    };
    if let Some(e) = known {
        return e.into();
    }
    record
        .and_then(|r| string(member(r, "format")))
        .filter(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".into())
}

/// A private folder for unpacked media, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("pm-cli-media-{}-{nanos:x}", std::process::id()));
        std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Set by SIGINT: the export stops at the next frame and removes its
/// temporary file.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

fn install_interrupt() {
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_interrupt as *const () as libc::sighandler_t,
        );
    }
}

/// The project, its media, and the composition facts a render needs.
struct Loaded {
    document: Arc<Document>,
    composition: String,
    media: MediaPaths,
    _scratch: Scratch,
}

fn load(options: &Options) -> Result<Loaded, Failure> {
    let bytes = std::fs::read(&options.input)
        .map_err(|e| usage(format!("{}: {e}", options.input.display())))?;
    let container = pm_model::container::read_container_bytes(
        &bytes,
        pm_model::container::ReadOptions::default(),
    )
    .map_err(|e| Failure::Failed(format!("{}: {e}", options.input.display())))?;
    let project = container.project().clone();
    let composition = match &options.composition {
        Some(c) => c.clone(),
        None => pm_model::canonical::open_comp_id(&project)
            .map(str::to_owned)
            .ok_or_else(|| Failure::Failed("the project has no open composition".into()))?,
    };
    let scratch = Scratch::new()?;
    let mut media = MediaPaths::default();
    let assets = member(&project, "assets");
    for (n, record) in container.media.iter().enumerate() {
        let id = record.id.to_string_lossy();
        let Some(data) = container.media_bytes(&bytes, record) else {
            continue;
        };
        let ext = extension_for(
            &record.media_type.to_string_lossy(),
            assets.and_then(|a| member(a, &id)),
        );
        let path = scratch.0.join(format!("{n}.{ext}"));
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        media.insert(id, path);
    }
    if let Some(Value::Object(map)) = assets {
        for (id, record) in map.iter() {
            let id = id.to_string_lossy();
            if media.paths.contains_key(&id) {
                continue;
            }
            if let Some(source) = string(member(record, "sourcePath")).map(PathBuf::from)
                && source.is_file()
            {
                media.insert(id, source);
            }
        }
    }
    for (asset, path) in &options.media {
        if !path.is_file() {
            return Err(usage(format!(
                "--media {asset}: {} is not a file",
                path.display()
            )));
        }
        media.insert(asset.clone(), path.clone());
    }
    Ok(Loaded {
        document: Arc::new(Document::new(Some(project), 1)),
        composition,
        media,
        _scratch: scratch,
    })
}

/// Export settings from the options and the composition, with today's
/// defaults: the composition's rate, whole range, scale 1 rounded to even
/// sizes, audio on, "Rec.709 (QuickTime and Safari)" tags, 32-bit precision,
/// dither on for 8-bit outputs.
fn settings(options: &Options, loaded: &Loaded) -> Result<ExportSettings, Failure> {
    let doc = &*loaded.document;
    let comp = pm_eval::Comp::resolve(doc, &loaded.composition)
        .map_err(|_| Failure::Failed(format!("composition {:?} not found", loaded.composition)))?;
    let (cw, ch) = composition_size(doc, &loaded.composition).map_err(|e| e.to_string())?;
    let preset = options.preset.as_deref().unwrap_or("master");
    let (mut format, mut alpha) = match preset {
        "master" => (
            ExportFormat::ProRes {
                profile: ProResProfile::Hq,
            },
            AlphaOutput::None,
        ),
        "master-alpha" => (
            ExportFormat::ProRes {
                profile: ProResProfile::P4444,
            },
            AlphaOutput::Straight,
        ),
        "web" => (ExportFormat::H264, AlphaOutput::None),
        "web-hevc" | "png" | "image-sequence" => {
            return Err(Failure::Unsupported(vec![format!(
                "preset {preset:?} needs HEVC or PNG output (Phase 3b)"
            )]));
        }
        other => {
            let known: Vec<String> = PRESETS
                .iter()
                .map(|(name, label)| format!("{name} ({label})"))
                .collect();
            return Err(usage(format!(
                "unknown preset {other:?}: {}",
                known.join(", ")
            )));
        }
    };
    if let Some(f) = &options.format {
        format = format_of(f)?;
        if !format.supports_alpha() {
            alpha = AlphaOutput::None;
        }
    }
    if let Some(a) = &options.alpha {
        alpha = match a.as_str() {
            "none" => AlphaOutput::None,
            "straight" => AlphaOutput::Straight,
            "premultiplied" => AlphaOutput::Premultiplied,
            other => return Err(usage(format!("alpha {other:?}"))),
        };
    }
    let quality = match options.quality.as_deref().unwrap_or("high") {
        "draft" => QualityTier::Draft,
        "high" => QualityTier::High,
        "max" => QualityTier::Max,
        other => return Err(usage(format!("quality {other:?}"))),
    };
    let frame_rate = match &options.fps {
        Some(f) => rate_of(f)?,
        None => {
            let rate = comp.rate().map(|r| r.rate).unwrap_or(pm_time::Rate::FPS_30);
            let (n, d) = (u32::try_from(rate.num()), u32::try_from(rate.den()));
            match (n, d) {
                (Ok(n), Ok(d)) => FrameRate::new(n, d),
                _ => return Err(Failure::Failed(format!("composition rate {rate}"))),
            }
        }
    };
    let duration = comp.duration();
    let (start, end) = match options.range.as_deref().unwrap_or("all") {
        "all" => (0.0, duration),
        "work" => {
            let work =
                member_array(doc, "work").ok_or_else(|| usage("the project has no work area"))?;
            (work.0, work.1)
        }
        other => {
            let (a, b) = other.split_once(':').ok_or_else(|| {
                usage(format!("range {other:?} (all, work, or START:END seconds)"))
            })?;
            let p = |s: &str| {
                s.parse::<f64>()
                    .map_err(|_| usage(format!("range {other:?}")))
            };
            (p(a)?, p(b)?)
        }
    };
    if !(end > start && start.is_finite() && end.is_finite()) {
        return Err(usage(format!("the range {start}..{end} is empty")));
    }
    let even = |v: f64| ((v / 2.0).round() * 2.0).max(2.0) as u32;
    let size = match (options.size, options.scale) {
        (Some((w, h)), _) => {
            if w % 2 != 0 || h % 2 != 0 {
                return Err(usage("--size must be even on both sides"));
            }
            Size::new(w, h)
        }
        (None, Some(s)) if (0.01..=16.0).contains(&s) => {
            Size::new(even(f64::from(cw) * s), even(f64::from(ch) * s))
        }
        (None, Some(s)) => return Err(usage(format!("scale {s} is outside 0.01 to 16"))),
        (None, None) => Size::new(even(f64::from(cw)), even(f64::from(ch))),
    };
    let color = match options.color.as_deref().unwrap_or("quicktime") {
        "quicktime" => OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        "broadcast" => OutputColor::Rec709Broadcast,
        other => return Err(usage(format!("color {other:?} (quicktime or broadcast)"))),
    };
    let precision = match options.precision.unwrap_or(32) {
        16 => Precision::Float16,
        32 => Precision::Float32,
        other => return Err(usage(format!("precision {other} (16 or 32)"))),
    };
    Ok(ExportSettings {
        format,
        alpha,
        quality,
        size,
        frame_rate,
        range: TickRange {
            start: seconds_to_ticks(start)?,
            end: seconds_to_ticks(end)?,
        },
        audio: !options.no_audio,
        motion_blur: false,
        color,
        precision,
        dither: options
            .dither
            .unwrap_or(matches!(format, ExportFormat::H264)),
        draft_from_proxies: false,
    })
}

fn member_array(doc: &Document, key: &str) -> Option<(f64, f64)> {
    let v = doc.root()?.get(key)?;
    let pm_model::NodeKind::Array(list) = v.kind() else {
        return None;
    };
    let n = |i: usize| match list.get(i)?.kind() {
        pm_model::NodeKind::Number(x) => Some(*x),
        _ => None,
    };
    Some((n(0)?, n(1)?)).filter(|(a, b)| b > a)
}

/// Runs `pm-cli render`. Returns the JSON summary printed on success.
pub fn run(options: &Options) -> Result<serde_json::Value, Failure> {
    let ext = options
        .output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let loaded = load(options)?;
    let settings = settings(options, &loaded)?;
    let container_ext = match settings.format {
        ExportFormat::ProRes { .. } => "mov",
        _ => "mp4",
    };
    if ext != container_ext {
        return Err(usage(format!(
            "{:?} is written to .{container_ext}, not .{ext}",
            settings.format
        )));
    }
    let output = absolute(&options.output)?;
    let job = ExportJob {
        document: loaded.document.clone(),
        composition: loaded.composition.clone(),
        settings,
        media: Arc::new(loaded.media.clone()),
        destination: output.clone(),
        overwrite: if options.force {
            Overwrite::Replace
        } else {
            Overwrite::Fail
        },
        mode: None,
        seed: 1,
    };
    install_interrupt();
    let mut renderer = Renderer::headless().map_err(|e| Failure::Failed(e.to_string()))?;
    let started = std::time::Instant::now();
    let mut last = -1i64;
    let mut progress = |p: Progress| {
        let pct = (p.fraction() * 100.0).floor() as i64;
        if pct / 5 != last / 5 {
            last = pct;
            eprintln!(
                "rendering: {pct}% ({} of {} frames)",
                p.frames_done, p.frames
            );
        }
    };
    let delivered = pm_export::render::run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &INTERRUPTED,
        &mut progress,
    )
    .map_err(|e| match e {
        JobError::Unsupported(list) => Failure::Unsupported(list),
        JobError::Cancelled => Failure::Failed("interrupted; nothing was written".into()),
        other => Failure::Failed(other.to_string()),
    })?;
    let r = &delivered.report;
    let seconds = started.elapsed().as_secs_f64();
    Ok(serde_json::json!({
        "path": delivered.path,
        "codec": r.codec,
        "profile": r.profile,
        "pixelFormat": r.pixel_format,
        "frames": r.frames,
        "size": [job.settings.size.width, job.settings.size.height],
        "frameRate": [job.settings.frame_rate.num, job.settings.frame_rate.den],
        "audioSamples": r.audio.as_ref().map(|a| a.samples),
        "colr": [r.cicp.0, r.cicp.1, r.cicp.2],
        "maxBlockErrorCv": r.max_pixel_error_cv(),
        "hardwareEncoder": r.hardware_encoder,
        "seconds": seconds,
        "fps": r.frames as f64 / seconds.max(1e-9),
    }))
}

fn absolute(p: &Path) -> Result<PathBuf, Failure> {
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    std::env::current_dir()
        .map(|d| d.join(p))
        .map_err(|e| Failure::Failed(e.to_string()))
}
