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
    /// Migration/extension fields survive every remember/preset round trip.
    #[serde(flatten)]
    pub opaque: BTreeMap<String, Value>,
}
impl Options {
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
        {
            return Err("invalid export settings".into());
        }
        Ok(())
    }
    /// Effective field support, without modifying dormant authored options.
    /// Engine JobSpec mapping must use this intent (ProRes 4444, H.264, VP9,
    /// PNG8; straight alpha; original media only). PMV uses save_project instead.
    pub fn intent(&self, has_audio: bool, caption_layers: usize) -> Result<Intent, String> {
        self.validate()?;
        let video = matches!(self.format, Format::Prores | Format::Mp4 | Format::Webm);
        let rendered = self.format != Format::Json;
        Ok(Intent {
            options: self.clone(),
            audio: video && has_audio && self.audio,
            straight_alpha: matches!(self.format, Format::Prores | Format::Png | Format::Still)
                && self.alpha,
            motion_blur: rendered && self.mblur,
            burn_captions: rendered && caption_layers > 0 && self.captions_burn,
            sidecar: if video && caption_layers > 0 {
                self.captions_sidecar.clone()
            } else {
                "none".into()
            },
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Intent {
    pub options: Options,
    pub audio: bool,
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
        use pm_protocol::priority::{
            AlphaOutput, ExportFormat, ProResProfile, QualityTier, WebmCodec,
        };
        use pm_protocol::validate::Validate;
        self.options.validate()?;
        planned.format = match self.options.format {
            Format::Json => return Ok(None),
            Format::Prores => ExportFormat::ProRes {
                profile: ProResProfile::P4444,
            },
            Format::Mp4 => ExportFormat::H264,
            Format::Webm => ExportFormat::Webm {
                codec: WebmCodec::Vp9,
            },
            Format::Png => ExportFormat::PngSequence { sixteen_bit: false },
            Format::Still => ExportFormat::PngStill { sixteen_bit: false },
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
        planned.draft_from_proxies = false;
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
