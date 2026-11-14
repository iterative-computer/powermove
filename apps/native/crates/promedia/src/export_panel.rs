//! Thin, worker-only export-sheet adapter. The application implements `ExportHost`
//! with kernel acknowledgements, native destination sheets and engine-service jobs.
//! There is no renderer, document writer, codec fallback or fake job in this module.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Prores,
    Mp4,
    Webm,
    Png,
    Still,
    Json,
}
impl Format {
    /// What this app exports, in menu order.
    pub const ALL: [Format; 6] = [
        Self::Prores,
        Self::Mp4,
        Self::Webm,
        Self::Png,
        Self::Still,
        Self::Json,
    ];
    /// The format's menu label (v1's).
    pub fn label(self) -> &'static str {
        match self {
            Self::Prores => "QuickTime · ProRes 4444",
            Self::Mp4 => "MP4 · H.264",
            Self::Webm => "WebM · VP9",
            Self::Png => "PNG Sequence",
            Self::Still => "Still Frame (PNG)",
            Self::Json => "Project File (.pmv)",
        }
    }
    pub fn video(self) -> bool {
        matches!(self, Self::Prores | Self::Mp4 | Self::Webm)
    }
    /// Whether an export in this format uses the option `field` (v1
    /// `exportFieldSupport`). The others stay stored, dormant, and show dimmed.
    pub fn reads(self, field: &str) -> bool {
        match field {
            "scale" | "fps" | "range" | "mblur" => self != Self::Json,
            "quality" => matches!(self, Self::Mp4 | Self::Webm),
            "audio" => self.video(),
            "alpha" => matches!(self, Self::Prores | Self::Png | Self::Still),
            "colorTag" => matches!(self, Self::Prores | Self::Mp4),
            // A timecode track and PCM are QuickTime's.
            "timecode" | "audioCoding" => self == Self::Prores,
            _ => true,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Prores => "mov",
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
            Self::Png | Self::Still => "png",
            Self::Json => "pmv",
        }
    }
}
/// The ProRes flavour of a QuickTime export. Older settings have none: 4444.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    #[serde(rename = "422proxy")]
    Proxy,
    #[serde(rename = "422lt")]
    Lt,
    #[serde(rename = "422")]
    Standard,
    #[serde(rename = "422hq")]
    Hq,
    #[default]
    #[serde(rename = "4444")]
    P4444,
    #[serde(rename = "4444xq")]
    P4444Xq,
}
impl Profile {
    fn engine(self) -> pm_protocol::priority::ProResProfile {
        use pm_protocol::priority::ProResProfile as P;
        match self {
            Self::Proxy => P::Proxy,
            Self::Lt => P::Lt,
            Self::Standard => P::Standard,
            Self::Hq => P::Hq,
            Self::P4444 => P::P4444,
            Self::P4444Xq => P::P4444Xq,
        }
    }
}
/// The codec of an MP4 export. HEVC is always Main 10.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    #[default]
    H264,
    Hevc,
}
fn eight() -> u8 {
    8
}
fn yes() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
}
/// How a delivery carries light. HDR needs an HDR project and a 10-bit codec;
/// elsewhere the choice stays dormant, like every other format-only field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DynamicRange {
    #[default]
    Sdr,
    Pq,
    Hlg,
}
impl DynamicRange {
    pub fn is_sdr(&self) -> bool {
        *self == Self::Sdr
    }
}
/// The label an SDR video carries. Either way the picture is the same codes:
/// the choice is what a player or a grading suite is told.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorTag {
    /// "Rec.709 (QuickTime and Safari)": sRGB transfer (1-13-1), as v1 writes.
    #[default]
    #[serde(rename = "rec709QuickTime")]
    QuickTime,
    /// "Rec.709": BT.709 transfer (1-1-1), as broadcast and colour-managed
    /// timelines expect.
    #[serde(rename = "rec709")]
    Rec709,
}
impl ColorTag {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
/// How a QuickTime export codes its sound. Older settings have none: 24-bit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCoding {
    #[serde(rename = "pcm16")]
    Pcm16,
    #[default]
    #[serde(rename = "pcm24")]
    Pcm24,
    #[serde(rename = "pcmFloat")]
    PcmFloat,
}
impl AudioCoding {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scale {
    Preset(f64),
    Custom(String),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub format: Format,
    pub scale: Scale,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub range: String,
    pub quality: String,
    pub mblur: bool,
    pub audio: bool,
    pub alpha: bool,
    pub captions_burn: bool,
    pub captions_sidecar: String,
    #[serde(default)]
    pub profile: Profile,
    #[serde(default)]
    pub codec: Codec,
    /// PNG bits per channel, 8 or 16.
    #[serde(default = "eight")]
    pub depth: u8,
    #[serde(default, skip_serializing_if = "DynamicRange::is_sdr")]
    pub dynamic_range: DynamicRange,
    /// Use Proxies (Draft): read proxy media where it exists. Otherwise an
    /// export always reads originals.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_proxies: bool,
    /// The colour label of an SDR ProRes or MP4.
    #[serde(default, skip_serializing_if = "ColorTag::is_default")]
    pub color_tag: ColorTag,
    /// The start timecode of a QuickTime export, `HH:MM:SS:FF` (`;FF` for
    /// drop-frame). Empty: the composition's own time at the first frame.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timecode: String,
    /// The sound coding of a QuickTime export.
    #[serde(default, skip_serializing_if = "AudioCoding::is_default")]
    pub audio_coding: AudioCoding,
    /// HLG HEVC: the Dolby Vision 8.4 metadata macOS's encoder adds. On
    /// unless the person turns it off (older options: on).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub dolby_vision: bool,
    /// Migration/extension fields survive every remember/preset round trip.
    #[serde(flatten)]
    pub opaque: BTreeMap<String, Value>,
}
impl Options {
    /// What a `width`×`height` composition at `fps` exports with, before any
    /// project defaults or remembered choices.
    pub fn new(width: u32, height: u32, fps: f64) -> Self {
        Self {
            format: Format::Mp4,
            scale: Scale::Preset(1.),
            width,
            height,
            fps,
            range: "work".into(),
            quality: "high".into(),
            mblur: true,
            audio: true,
            alpha: false,
            captions_burn: false,
            captions_sidecar: "none".into(),
            profile: Profile::default(),
            codec: Codec::default(),
            depth: eight(),
            dynamic_range: DynamicRange::default(),
            use_proxies: false,
            color_tag: ColorTag::default(),
            timecode: String::new(),
            audio_coding: AudioCoding::default(),
            dolby_vision: true,
            opaque: BTreeMap::new(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        // Portable project export does not read rendering fields. Retain dormant
        // settings even when they cannot describe an encodable raster.
        if self.format == Format::Json && self.fps.is_finite() {
            return Ok(());
        }
        let scale = match &self.scale {
            Scale::Preset(n) => [0.5, 1., 2.].contains(n),
            Scale::Custom(s) => s == "custom",
        };
        if !scale
            || !(16..=8192).contains(&self.width)
            || !(16..=8192).contains(&self.height)
            || !self.fps.is_finite()
            || self.fps <= 0.
            || !["work", "all"].contains(&self.range.as_str())
            || !["draft", "high", "max"].contains(&self.quality.as_str())
            || !["none", "srt", "vtt"].contains(&self.captions_sidecar.as_str())
            || ![8, 16].contains(&self.depth)
        {
            return Err("invalid export settings".into());
        }
        Ok(())
    }
    /// The start timecode as the engine takes it, checked against the export
    /// rate: every field in range, drop-frame only where the rate has it.
    fn start_timecode(&self) -> Result<pm_protocol::priority::ExportTimecode, String> {
        let rate = pm_time::rate_from_document(None, self.fps)
            .map_err(|e| e.to_string())?
            .rate;
        let drop_frame = self.timecode.contains(';');
        let format = if drop_frame {
            pm_time::TimecodeFormat::drop_frame(rate)
                .map_err(|_| "Drop-frame timecode (;) needs 29.97 or 59.94 fps.".to_string())?
        } else {
            pm_time::TimecodeFormat::non_drop(rate)
        };
        format
            .parse(&self.timecode)
            .map_err(|_| "Start timecode must be HH:MM:SS:FF at this frame rate.".to_string())?;
        Ok(pm_protocol::priority::ExportTimecode {
            start: self.timecode.clone(),
            drop_frame,
            reel: None,
        })
    }
    /// Effective field support, without modifying dormant authored options.
    /// Engine JobSpec mapping must use this intent (straight alpha, original
    /// media only). PMV uses save_project instead.
    pub fn intent(&self, has_audio: bool, caption_layers: usize) -> Result<Intent, String> {
        self.validate()?;
        let f = self.format;
        let audio = f.reads("audio") && has_audio && self.audio;
        let dynamic_range = if matches!(self.format, Format::Prores | Format::Mp4) {
            self.dynamic_range
        } else {
            DynamicRange::Sdr
        };
        Ok(Intent {
            options: self.clone(),
            audio,
            dynamic_range,
            color_tag: if f.reads("colorTag") && dynamic_range.is_sdr() {
                self.color_tag
            } else {
                ColorTag::default()
            },
            timecode: if f.reads("timecode") && !self.timecode.is_empty() {
                Some(self.start_timecode()?)
            } else {
                None
            },
            audio_coding: if f.reads("audioCoding") && audio {
                self.audio_coding
            } else {
                AudioCoding::default()
            },
            straight_alpha: f.reads("alpha")
                && (f != Format::Prores || self.profile.engine().supports_alpha())
                && self.alpha,
            motion_blur: f.reads("mblur") && self.mblur,
            burn_captions: f != Format::Json && caption_layers > 0 && self.captions_burn,
            sidecar: if f.video() && caption_layers > 0 {
                self.captions_sidecar.clone()
            } else {
                "none".into()
            },
        })
    }
}
/// v1 `EXPORT_FORMAT_OPTIONS`: a project's stored default format is kept
/// even where this app has no encoder for it (`rec`, `web`).
const V1_FORMATS: [&str; 8] = [
    "prores", "mp4", "webm", "rec", "png", "still", "web", "json",
];
/// JavaScript `Number(value)` for a JSON value.
fn js_number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.,
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                0.
            } else {
                text.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}
/// A project's stored export defaults as v1 `normalizeExportDefaults(raw,
/// fallbackFps)` reads them, field for field.
pub fn normalize_defaults(stored: &Value, fallback_fps: f64) -> Value {
    let pick = |key: &str, allowed: &[&str], fallback: &str| {
        let value = stored[key].as_str().filter(|v| allowed.contains(v));
        serde_json::json!(value.unwrap_or(fallback))
    };
    let flag = |key: &str, fallback: bool| stored[key].as_bool().unwrap_or(fallback);
    let base_fps = if fallback_fps.is_finite() && fallback_fps > 0. {
        fallback_fps.min(240.)
    } else {
        30.
    };
    let fps = js_number(&stored["fps"]);
    let scale = js_number(&stored["scale"]);
    let number = |n: f64| serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number);
    serde_json::json!({
        "format": pick("format", &V1_FORMATS, "mp4"),
        "scale": number(if !scale.is_finite() || scale <= 0. { 1. } else { scale.clamp(0.01, 16.) }),
        "fps": number(if fps.is_finite() && fps > 0. { fps.min(240.) } else { base_fps }),
        "range": pick("range", &["work", "all"], "work"),
        "quality": pick("quality", &["draft", "high", "max"], "high"),
        "mblur": flag("mblur", true),
        "audio": flag("audio", true),
        "alpha": flag("alpha", false),
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct Intent {
    pub options: Options,
    pub audio: bool,
    pub dynamic_range: DynamicRange,
    pub color_tag: ColorTag,
    pub timecode: Option<pm_protocol::priority::ExportTimecode>,
    pub audio_coding: AudioCoding,
    pub straight_alpha: bool,
    pub motion_blur: bool,
    pub burn_captions: bool,
    pub sidecar: String,
}
impl Intent {
    /// Apply panel intent to native planner settings for JobSpec::Export.
    /// The planner owns tick range, output size, colour, precision and audio
    /// index. They are retained verbatim. Sidecars stay explicit in `self` and
    /// must be written/verified by the host before publishing completion.
    pub fn engine_settings(
        &self,
        mut planned: pm_protocol::priority::ExportSettings,
    ) -> Result<Option<pm_protocol::priority::ExportSettings>, String> {
        use pm_protocol::priority::{AlphaOutput, Container, ExportFormat, QualityTier, WebmCodec};
        use pm_protocol::validate::Validate;
        self.options.validate()?;
        let sixteen_bit = self.options.depth == 16;
        planned.format = match self.options.format {
            Format::Json => return Ok(None),
            Format::Prores => ExportFormat::ProRes {
                profile: self.options.profile.engine(),
            },
            // H.264 has no HDR profile on VideoToolbox: HDR delivers HEVC Main 10.
            Format::Mp4 if !self.dynamic_range.is_sdr() => ExportFormat::Hevc {
                ten_bit: true,
                container: Container::Mp4,
            },
            Format::Mp4 => match self.options.codec {
                Codec::H264 => ExportFormat::H264,
                Codec::Hevc => ExportFormat::Hevc {
                    ten_bit: true,
                    container: Container::Mp4,
                },
            },
            Format::Webm => ExportFormat::Webm {
                codec: WebmCodec::Vp9,
            },
            Format::Png => ExportFormat::PngSequence { sixteen_bit },
            Format::Still => ExportFormat::PngStill { sixteen_bit },
        };
        planned.color = match self.dynamic_range {
            DynamicRange::Sdr if self.color_tag == ColorTag::Rec709 => {
                pm_protocol::color::OutputColor::Rec709Broadcast
            }
            DynamicRange::Sdr => planned.color,
            DynamicRange::Pq => pm_protocol::color::OutputColor::Rec2100Pq,
            DynamicRange::Hlg => pm_protocol::color::OutputColor::Rec2100Hlg,
        };
        planned.alpha = if self.straight_alpha {
            AlphaOutput::Straight
        } else {
            AlphaOutput::None
        };
        planned.quality = match self.options.quality.as_str() {
            "draft" => QualityTier::Draft,
            "high" => QualityTier::High,
            "max" => QualityTier::Max,
            _ => return Err("invalid export quality".into()),
        };
        planned.audio = self.audio;
        planned.motion_blur = self.motion_blur;
        // Only where the sheet offers the choice: a still or a project file
        // never carries a hidden draft setting from another format.
        planned.draft_from_proxies = self.options.use_proxies
            && matches!(
                self.options.format,
                Format::Prores | Format::Mp4 | Format::Webm | Format::Png
            );
        // HLG HEVC only: macOS adds Dolby Vision 8.4 there.
        planned.delivery.without_dolby_vision = !self.options.dolby_vision
            && self.options.format == Format::Mp4
            && self.dynamic_range == DynamicRange::Hlg;
        planned.delivery.timecode = self.timecode.clone();
        planned.delivery.audio_coding = match self.audio_coding {
            AudioCoding::Pcm24 => None,
            AudioCoding::Pcm16 => Some(pm_protocol::priority::AudioCoding::Pcm16),
            AudioCoding::PcmFloat => Some(pm_protocol::priority::AudioCoding::PcmFloat),
        };
        planned.delivery.hide_captions = !self.burn_captions;
        planned.delivery.preview = true;
        // Still-frame timing is entirely the planner's (current playhead).
        if self.options.format != Format::Still {
            let rate = pm_time::rate_from_document(None, self.options.fps)
                .map_err(|e| e.to_string())?
                .rate;
            planned.frame_rate = pm_protocol::common::FrameRate::new(
                u32::try_from(rate.num()).map_err(|_| "export rate numerator overflow")?,
                u32::try_from(rate.den()).map_err(|_| "export rate denominator overflow")?,
            );
        }
        planned.validate().map_err(|e| e.to_string())?;
        Ok(Some(planned))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    File(PathBuf),
    Folder(PathBuf),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeginResult<J> {
    DestinationCancelled,
    Started(J),
}
/// Implement on a worker; UI enqueues requests and never calls these inline.
/// A host must reserve the per-project export slot before entering `begin` and
/// release it on cancelled destination/error. Capability checks precede all work.
pub trait ExportHost {
    /// A pin of the actual kernel snapshot/revision, NOT a JS document mirror.
    type Frozen;
    type Job;
    fn available(&self, format: Format) -> Result<(), String>;
    /// Preserve unknown defaults and archive prior same-name presets. Wait only
    /// on the worker for the real actor ack, including durability errors.
    fn remember(&mut self, options: &Options) -> Result<Self::Frozen, String>;
    /// NSSavePanel or NSOpenPanel (PNG folder), nested on the Export sheet.
    fn choose_destination(
        &mut self,
        suggested: &str,
        folder: bool,
    ) -> Result<Option<Destination>, String>;
    /// Freeze intent into engine-service ExportRequest/JobSpec::Export; register
    /// TaskKind::Export in BackgroundTasks, start_scheduled with interactive
    /// activity, originals only, verified no-clobber publication. This method
    /// must refuse unsupported captions/resources rather than omit them.
    fn render(
        &mut self,
        frozen: Self::Frozen,
        intent: Intent,
        destination: Destination,
    ) -> Result<Self::Job, String>;
    /// PMV4 portable writer with original-media closure; no render job/encoder.
    fn save_project(
        &mut self,
        frozen: Self::Frozen,
        destination: PathBuf,
    ) -> Result<Self::Job, String>;
}
/// Implements B440/A264: remember → destination → work. Cancelling the picker
/// never touches engine resources; a failure never becomes success-shaped data.
pub fn begin<H: ExportHost>(
    host: &mut H,
    name: &str,
    options: &Options,
    has_audio: bool,
    caption_layers: usize,
) -> Result<BeginResult<H::Job>, String> {
    let intent = options.intent(has_audio, caption_layers)?;
    host.available(options.format)?;
    let frozen = host.remember(options)?;
    let folder = options.format == Format::Png;
    let suggested = format!("{name}.{}", options.format.extension());
    let Some(destination) = host.choose_destination(&suggested, folder)? else {
        return Ok(BeginResult::DestinationCancelled);
    };
    match (&destination, folder) {
        (Destination::File(path), false) | (Destination::Folder(path), true)
            if !path.as_os_str().is_empty() => {}
        _ => return Err("destination kind does not match export format".into()),
    }
    let job = if options.format == Format::Json {
        let Destination::File(path) = destination else {
            unreachable!("validated file destination")
        };
        host.save_project(frozen, path)?
    } else {
        host.render(frozen, intent, destination)?
    };
    Ok(BeginResult::Started(job))
}
