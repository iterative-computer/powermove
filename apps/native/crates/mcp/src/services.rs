//! Worker-only service boundaries. The app injects its engine/export child;
//! standalone builds fail explicitly when no real host is installed.
use crate::{Automation, CaptureRequest, Error, Result};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct FrozenProject {
    pub project_id: String,
    pub document_id: u64,
    pub revision: u64,
    pub project: Value,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaOperation {
    Probe,
    SampleFrames,
    ContactSheet,
    Waveform,
}
impl MediaOperation {
    pub fn tool(&self) -> &'static str {
        match self {
            Self::Probe => "probe_media",
            Self::SampleFrames => "sample_media_frames",
            Self::ContactSheet => "media_contact_sheet",
            Self::Waveform => "media_waveform",
        }
    }
}
#[derive(Clone, Debug)]
pub struct MediaRequest {
    pub operation: MediaOperation,
    pub snapshot: FrozenProject,
    /// Frozen schema arguments; times for layer targets are composition time.
    /// The engine must evaluate timing/remapping from this exact snapshot.
    pub arguments: Value,
    pub asset: Option<Value>,
    pub layer: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct Image {
    /// MCP uses base64, never a local path or a data URL.
    pub data: String,
    pub mime_type: String,
}
#[derive(Clone, Debug)]
pub struct ToolOutput {
    pub value: Value,
    pub images: Vec<Image>,
}
impl ToolOutput {
    pub fn text(value: Value) -> Self {
        Self {
            value,
            images: vec![],
        }
    }
    pub fn content(self) -> Result<Value> {
        let mut content = vec![
            json!({"type":"text","text":pm_model::json::to_canonical_string(&crate::model(&self.value))}),
        ];
        for image in self.images {
            let bytes = image.data.as_bytes();
            let padding = bytes.iter().rev().take_while(|b| **b == b'=').count();
            if !["image/png", "image/jpeg"].contains(&image.mime_type.as_str())
                || bytes.is_empty()
                || bytes.len() % 4 != 0
                || padding > 2
                || !bytes[..bytes.len() - padding]
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/".contains(b))
            {
                return Err(Error::invalid("Engine returned invalid MCP image content"));
            }
            content.push(json!({"type":"image","data":image.data,"mimeType":image.mime_type}));
        }
        Ok(json!(content))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct VideoOptions {
    pub name: Option<String>,
    pub format: String,
    pub range: String,
    pub audio: bool,
    pub motion_blur: bool,
    pub alpha: bool,
    pub studio_quality: Option<Value>,
    pub studio_render_scale: Option<f32>,
    /// The Export sheet's own options an agent gave, under the sheet's
    /// names: laid over this export's defaults and checked by the same
    /// planner as a person's export.
    pub sheet: serde_json::Map<String, Value>,
}
/// The Export sheet's options an agent gives under the sheet's own names.
pub const SHEET_OPTIONS: [&str; 15] = [
    "profile",
    "codec",
    "quality",
    "scale",
    "width",
    "height",
    "fps",
    "dynamicRange",
    "colorTag",
    "timecode",
    "audioCoding",
    "useProxies",
    "captionsBurn",
    "captionsSidecar",
    "dolbyVision",
];
impl VideoOptions {
    pub fn parse(a: &Value) -> Result<Self> {
        let format = a["format"].as_str().unwrap_or("mp4").to_owned();
        let alpha = a["alpha"].as_bool().unwrap_or(false);
        if alpha && format != "prores" {
            return Err(Error::invalid("Transparent video requires ProRes."));
        }
        if alpha && !a["profile"].as_str().unwrap_or("4444").starts_with("4444") {
            return Err(Error::invalid(
                "Transparent video requires ProRes 4444 or 4444 XQ.",
            ));
        }
        let sheet: serde_json::Map<String, Value> = SHEET_OPTIONS
            .iter()
            .filter_map(|k| a.get(*k).map(|v| ((*k).to_owned(), v.clone())))
            .collect();
        let sized = sheet.contains_key("width") || sheet.contains_key("height");
        let custom = sheet.get("scale").and_then(Value::as_str) == Some("custom");
        if (sized || custom) && !(custom || !sheet.contains_key("scale"))
            || (sized || custom) && !(sheet.contains_key("width") && sheet.contains_key("height"))
        {
            return Err(Error::invalid(
                "A custom size takes scale \"custom\" with both width and height.",
            ));
        }
        let studio_render_scale = a
            .get("studioRenderScale")
            .map(|v| {
                v.as_f64()
                    .filter(|v| v.is_finite() && *v > 0. && *v <= 1.)
                    .map(|v| v as f32)
                    .filter(|v| *v > 0.)
                    .ok_or_else(|| {
                        Error::invalid(
                            "Studio render scale must be greater than zero and at most one",
                        )
                    })
            })
            .transpose()?;
        Ok(Self {
            name: a["name"].as_str().map(str::to_owned),
            format,
            range: a["range"].as_str().unwrap_or("all").into(),
            audio: a["audio"].as_bool().unwrap_or(true),
            motion_blur: a["motionBlur"].as_bool().unwrap_or(true),
            alpha,
            studio_quality: a.get("studioQuality").cloned(),
            studio_render_scale,
            sheet,
        })
    }
}
#[derive(Clone, Debug)]
pub struct VideoExportRequest {
    pub snapshot: FrozenProject,
    pub options: VideoOptions,
    /// Explicit isolated destination. Host must use atomic no-clobber publish,
    /// originals-only media (unless separately authorized), and reap its child.
    pub directory: PathBuf,
}
/// Runs on the MCP worker, never the UI thread. Video adapters own child
/// cancellation/draining. Caption export is pure text and needs no subprocess.
/// A worker copies an original into durable media and admits it through the kernel.
#[derive(Clone, Debug)]
pub struct MediaImport {
    pub frozen: FrozenProject,
    pub path: PathBuf,
    pub folder: Option<String>,
    pub replace: Option<String>,
    pub expected_fingerprint: Option<String>,
    pub writer: u32,
    /// An image sequence (`path` its folder or one of its files) at this
    /// frame rate, as File ▸ Import Image Sequence imports it.
    pub sequence: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct ImportedMedia {
    pub status: &'static str,
    pub asset: String,
    pub name: String,
    pub kind: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub duration: Option<f64>,
    pub has_audio: bool,
    pub fingerprint: String,
    pub revision: u64,
    /// What else the import found (an image sequence's missing frames).
    pub note: Option<String>,
}
pub trait MediaImporter: Send + Sync {
    fn import(&self, request: &MediaImport) -> Result<ImportedMedia>;
}
/// Speech to text on this Mac, as the app's transcription service.
pub trait Speech: Send + Sync {
    /// `transcribe_media`'s answer: the transcript page, `transcribing` with
    /// progress while a job runs, or `model-required`.
    fn transcribe(&self, snapshot: &FrozenProject, args: &Value) -> Result<Value>;
    /// `generate_captions`: `{status:"running", jobId, progress}` while it
    /// transcribes, then `{status:"done", command}` with the `add_captions`
    /// command the session applies in its own transaction.
    fn captions(&self, snapshot: &FrozenProject, args: &Value, owner: u32) -> Result<Value>;
}
/// Reads the durable agent thread archive, whose format the app owns.
pub trait ThreadArchive: Send + Sync {
    fn read(&self, tool: &str, project: Option<&str>, args: &Value) -> Result<Value>;
}
/// What an engine export delivered.
#[derive(Clone, Debug)]
pub struct VideoDelivered {
    pub path: PathBuf,
    /// The file's size and frame rate (`None`: the project's).
    pub size: Option<(u32, u32)>,
    pub fps: Option<f64>,
    /// What the person would be told with the file: sound clipped, fine
    /// detail the data rate did not keep.
    pub notes: Vec<String>,
}
impl From<PathBuf> for VideoDelivered {
    fn from(path: PathBuf) -> Self {
        Self {
            path,
            size: None,
            fps: None,
            notes: vec![],
        }
    }
}
pub trait ExportChild: Send + Sync {
    fn video(&self, request: &VideoExportRequest) -> Result<VideoDelivered>;
    fn video_progress(&self, _: &FrozenProject) -> Option<crate::export_jobs::Progress> {
        None
    }
    fn captions(&self, layer: &Value, format: &str) -> Result<String> {
        captions(layer, format)
    }
}
pub struct NativeExportChild;
impl ExportChild for NativeExportChild {
    fn video(&self, _: &VideoExportRequest) -> Result<VideoDelivered> {
        Err(Error::unavailable(
            "pm-engine-service export child, original media resolver, fonts and extension definitions",
        ))
    }
}
impl Automation {
    pub fn freeze(&self, id: &str) -> Result<FrozenProject> {
        Ok(FrozenProject {
            project_id: id.into(),
            document_id: self.view(id)?.document,
            revision: self.revision(id)?,
            project: self.read_document(id)?,
        })
    }
    pub fn render(&self, id: &str, args: &Value) -> Result<ToolOutput> {
        let snapshot = self.freeze(id)?;
        let p = &snapshot.project;
        let duration = p["dur"]
            .as_f64()
            .filter(|v| *v != 0.)
            .unwrap_or(10.)
            .max(0.1);
        let safe = |values: Vec<f64>| {
            let mut out = vec![];
            for value in values {
                let t = (value.clamp(0., duration) * 1000.).round() / 1000.;
                if !out.contains(&t) {
                    out.push(t)
                }
                if out.len() == 5 {
                    break;
                }
            }
            out
        };
        let mut times = args["times"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect::<Vec<_>>();
        if times.is_empty() {
            let selection = self.view(id)?.selection.layers().to_vec();
            let selected = p["layers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|l| {
                    l["id"]
                        .as_str()
                        .is_some_and(|s| selection.iter().any(|id| id == s))
                })
                .collect::<Vec<_>>();
            let from = selected
                .iter()
                .filter_map(|l| l["from"].as_f64())
                .reduce(f64::min)
                .unwrap_or(p["work"][0].as_f64().unwrap_or(0.));
            let to = selected
                .iter()
                .map(|l| l["from"].as_f64().unwrap_or(0.) + l["dur"].as_f64().unwrap_or(0.))
                .reduce(f64::max)
                .unwrap_or(p["work"][1].as_f64().unwrap_or(duration));
            times = vec![
                self.playhead(id),
                from,
                from + (to - from) * 0.5,
                from.max(to - 1. / p["fps"].as_f64().unwrap_or(30.)),
            ];
        }
        let times = safe(times);
        let width = args["width"].as_u64().unwrap_or(720) as u32;
        let request = CaptureRequest {
            project_id: id.into(),
            revision: snapshot.revision,
            times: times.clone(),
            width,
        };
        let engine=self.engine.as_ref().ok_or_else(||Error::unavailable("pm-engine-service Headless frame renderer, encoded PNG/JPEG, fonts and original media resolver"))?;
        let result = engine.capture_snapshot(&request, &snapshot)?;
        // Capture is revision/document-bound even if a host completes work late.
        self.check_frozen(&snapshot)?;
        let urls = result["images"]
            .as_array()
            .ok_or_else(|| Error::invalid("Engine capture must return images"))?;
        if urls.len() != times.len() {
            return Err(Error::invalid(
                "Engine capture did not return every requested frame",
            ));
        }
        let images = urls
            .iter()
            .map(|v| {
                let url = v
                    .as_str()
                    .ok_or_else(|| Error::invalid("Engine capture must return data URLs"))?;
                let (header, data) = url
                    .split_once(",")
                    .ok_or_else(|| Error::invalid("Invalid capture data URL"))?;
                let mime = header
                    .strip_prefix("data:")
                    .and_then(|v| v.strip_suffix(";base64"))
                    .ok_or_else(|| Error::invalid("Invalid capture data URL"))?;
                Ok(Image {
                    data: data.into(),
                    mime_type: mime.into(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ToolOutput {
            value: json!({"times":times,"width":width,"rendered":images.len()}),
            images,
        })
    }
    fn check_frozen(&self, p: &FrozenProject) -> Result<()> {
        let view = self.view(&p.project_id)?;
        if view.document != p.document_id || view.revision != p.revision {
            return Err(Error::conflict(
                "The composition changed during engine work",
            ));
        }
        Ok(())
    }
    pub fn media_tool(
        &self,
        id: &str,
        operation: MediaOperation,
        args: &Value,
    ) -> Result<ToolOutput> {
        let snapshot = self.freeze(id)?;
        let p = &snapshot.project;
        let composition =
            operation == MediaOperation::ContactSheet && args["target"] == "composition";
        let (asset, layer) = if composition {
            (None, None)
        } else {
            let layer = args["layerId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|id| {
                    p["layers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|l| {
                            l["id"] == id
                                && ["video", "audio"].contains(&l["type"].as_str().unwrap_or(""))
                        })
                        .cloned()
                        .ok_or_else(|| {
                            Error::invalid("Choose a media layer from get_project_state.")
                        })
                })
                .transpose()?;
            let asset_id = layer
                .as_ref()
                .and_then(|l| l["d"]["asset"].as_str())
                .or_else(|| args["assetId"].as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::invalid("Supply assetId or layerId."))?;
            if let Some(explicit) = args["assetId"].as_str().filter(|s| !s.is_empty())
                && explicit != asset_id
            {
                return Err(Error::invalid(
                    "The selected layer plays a different asset. Pass one or the other.",
                ));
            }
            let asset = p["assets"]
                .get(asset_id)
                .filter(|v| v.is_object())
                .cloned()
                .ok_or_else(|| Error::invalid("Choose an existing assetId from list_media."))?;
            (Some(asset), layer)
        };
        let engine = self.engine.as_ref().ok_or_else(|| {
            Error::unavailable(format!(
                "pm-engine-service {} worker, original media resolver and encoded results",
                operation.tool()
            ))
        })?;
        let result = crate::media_tools::run(
            engine.as_ref(),
            &MediaRequest {
                operation,
                snapshot: snapshot.clone(),
                arguments: args.clone(),
                asset,
                layer,
            },
        )?;
        self.check_frozen(&snapshot)?;
        Ok(result)
    }
    pub fn export_video(&self, id: &str, args: &Value) -> Result<Value> {
        let options = VideoOptions::parse(args)?;
        let snapshot = self.freeze(id)?;
        run_video(
            self.export_child.as_ref(),
            &VideoExportRequest {
                snapshot,
                options,
                directory: self.exports.clone(),
            },
        )
    }
    pub fn export_captions(&self, id: &str, args: &Value) -> Result<Value> {
        let p = self.read_document(id)?;
        let layer = p["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["id"] == args["layerId"] && l["type"] == "captions")
            .ok_or_else(|| Error::invalid("Read a captions layer ID from get_project_state."))?;
        let format = args["format"].as_str().unwrap_or("srt");
        let text = self.export_child.captions(layer, format)?;
        Ok(
            json!({"layerId":layer["id"],"format":format,"cues":layer["d"]["cues"].as_array().map_or(0,Vec::len),"text":text}),
        )
    }
}
fn round(n: f64) -> f64 {
    (n * 1000.).round() / 1000.
}
fn stamp(n: f64, separator: char) -> String {
    let ms = (n * 1000.).round().max(0.) as u64;
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        ms / 3600000,
        ms / 60000 % 60,
        ms / 1000 % 60,
        separator,
        ms % 1000
    )
}
/// Normalise the tagged caption text/times, sort/de-overlap, then clip to the
/// visible layer interval. IDs/styles/words do not affect exported text.
pub fn captions(layer: &Value, format: &str) -> Result<String> {
    let mut cues = vec![];
    let spaces = regex::Regex::new(r"[\t ]+").unwrap();
    for cue in layer["d"]["cues"].as_array().into_iter().flatten() {
        let finite = |v: Option<&Value>| -> Option<f64> {
            let v = v?;
            if v.as_str()
                .is_some_and(|s| pm_model::json::js_trim(s).is_empty())
            {
                return None;
            }
            let n = match v {
                Value::Null => 0.,
                Value::Bool(b) => {
                    if *b {
                        1.
                    } else {
                        0.
                    }
                }
                Value::Number(n) => n.as_f64()?,
                _ => pm_model::json::string_to_number(
                    &crate::model(v).to_js_string().to_string_lossy(),
                ),
            };
            n.is_finite().then_some(n)
        };
        let (Some(mut start), Some(mut end)) = (finite(cue.get("start")), finite(cue.get("end")))
        else {
            continue;
        };
        if end < start {
            std::mem::swap(&mut start, &mut end)
        }
        start = start.max(0.);
        end = end.max(0.);
        if end - start < 0.04 - 1e-6 {
            continue;
        }
        let raw = cue
            .get("text")
            .filter(|v| !v.is_null())
            .map(|v| crate::model(v).to_js_string().to_string_lossy())
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let text = raw
            .lines()
            .map(|s| pm_model::json::js_trim(&spaces.replace_all(s, " ")).to_owned())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            cues.push(((start * 1e6).round() / 1e6, (end * 1e6).round() / 1e6, text));
        }
    }
    cues.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut normal: Vec<(f64, f64, String)> = vec![];
    for cue in cues {
        if let Some(prev) = normal.last_mut()
            && cue.0 < prev.1 - 1e-6
        {
            if cue.0 - prev.0 < 0.04 {
                prev.2.push('\n');
                prev.2.push_str(&cue.2);
                prev.1 = prev.1.max(cue.1);
                continue;
            } else {
                prev.1 = cue.0;
            }
        }
        normal.push(cue);
    }
    let from = layer["from"].as_f64().unwrap_or(0.);
    let to = from + layer["dur"].as_f64().unwrap_or(0.);
    let mut blocks = vec![];
    for (start, end, text) in normal {
        let start = (start + from).max(from);
        let end = (end + from).min(to);
        if end - start < 0.02 {
            continue;
        }
        let start = round(start);
        let end = round(end).max(start + 0.001);
        let text = if format == "vtt" {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        } else {
            text
        };
        let separator = if format == "vtt" { '.' } else { ',' };
        let prefix = if format == "vtt" {
            String::new()
        } else {
            format!("{}\n", blocks.len() + 1)
        };
        blocks.push(format!(
            "{prefix}{} --> {}\n{text}\n",
            stamp(start, separator),
            stamp(end, separator)
        ));
    }
    if format == "vtt" {
        let language = layer["d"]["language"].as_str().filter(|s| {
            regex::Regex::new(r"^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$")
                .unwrap()
                .is_match(s)
        });
        let header =
            language.map_or_else(|| "WEBVTT\n".into(), |l| format!("WEBVTT\nLanguage: {l}\n"));
        blocks.insert(0, header);
    }
    Ok(blocks.join("\n"))
}

pub(crate) fn run_video(child: &dyn ExportChild, request: &VideoExportRequest) -> Result<Value> {
    let delivered = child.video(request)?;
    let path = &delivered.path;
    let canonical = std::fs::canonicalize(&path)?;
    let directory = std::fs::canonicalize(&request.directory)?;
    if !canonical.starts_with(directory)
        || !std::fs::metadata(&canonical)?.is_file()
        || std::fs::metadata(&canonical)?.len() == 0
    {
        return Err(Error::invalid(
            "Export produced no saved file in the explicit export directory.",
        ));
    }
    let project = &request.snapshot.project;
    let (width, height) = delivered
        .size
        .map_or((project["w"].clone(), project["h"].clone()), |(w, h)| {
            (json!(w), json!(h))
        });
    let fps = delivered.fps.map_or(project["fps"].clone(), |f| json!(f));
    let mut result = json!({"path":path,"projectId":request.snapshot.project_id,"width":width,"height":height,"fps":fps});
    // v1's result, with what the person would be told when there is any.
    if !delivered.notes.is_empty() {
        result["notes"] = json!(delivered.notes);
    }
    Ok(result)
}
