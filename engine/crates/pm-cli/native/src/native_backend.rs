//! macOS adapter: in-process frame service and same-binary export child.
//! No window, Chromium or Node. The CLI pumps a native font broker on main.
use super::*;
use pm_engine_service::{
    Headless, MediaCatalog, Scene,
    jobs::{ExportHandle, ExportRequest},
};
use pm_protocol::{
    color::{OutputColor, Precision, QuickTimeTag},
    common::{FrameRate, SessionId, Size, TickRange, Ticks},
    priority::{
        AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, ProResProfile, QualityTier,
    },
};

#[derive(Default)]
pub struct Native {
    headless: Option<Headless>,
    media: Option<MediaCatalog>,
    fonts: Option<pm_render::text::host::RenderFonts>,
}
impl Native {
    pub fn with_fonts(fonts: pm_render::text::host::RenderFonts) -> Self {
        Self {
            fonts: Some(fonts),
            ..Self::default()
        }
    }
    fn media(&mut self, project: &Project) -> Result<MediaCatalog> {
        if let Some(media) = &self.media {
            return Ok(media.clone());
        }
        let p = project.published()?;
        // A project-scoped font loader is a separate host capability. Never
        // substitute a system font for a known embedded project font.
        if project.document()?["assets"]
            .as_object()
            .is_some_and(|assets| {
                assets.values().any(|a| {
                    a["kind"] == "font"
                        || a["type"]
                            .as_str()
                            .is_some_and(|t| t.starts_with("font/") || t.contains("font"))
                })
            })
        {
            return Err(Error::unavailable("project-scoped font asset activation"));
        }
        let mut media = MediaCatalog::default();
        for (index, item) in p.media.iter().enumerate() {
            let destination = project.cache_dir().join(format!("asset-{index}"));
            item.copy_to(&destination).map_err(Error::kernel)?;
            media.originals.insert(item.id.to_string(), destination);
        }
        let doc = project.document()?;
        if let Some(assets) = doc["assets"].as_object() {
            for (id, asset) in assets {
                if media.originals.contains_key(id) {
                    continue;
                }
                let path = asset
                    .get("sourcePath")
                    .or_else(|| asset.get("path"))
                    .and_then(Value::as_str)
                    .map(Path::new)
                    .map(|path| {
                        if path.is_absolute() {
                            path.to_path_buf()
                        } else {
                            project.source.parent().expect("source parent").join(path)
                        }
                    });
                if let Some(path) = path.filter(|p| p.is_file()) {
                    media.originals.insert(id.clone(), path);
                }
            }
        }
        self.media = Some(media.clone());
        Ok(media)
    }
}
fn native_error(e: impl std::fmt::Display) -> Error {
    error("engine_error", e.to_string())
}
fn ticks(seconds: f64) -> Result<Ticks> {
    let t = pm_time::ticks_from_seconds(seconds).map_err(native_error)?;
    Ticks::new(t).map_err(native_error)
}
impl FrameEngine for Native {
    fn frame(&mut self, project: &Project, request: &FrameSpec) -> Result<Image> {
        let p = project.published()?;
        let media = self.media(project)?;
        if self.headless.is_none() {
            let fonts = self.fonts.take().ok_or_else(|| {
                Error::unavailable("native frame capture requires a main-thread font broker")
            })?;
            self.headless = Some(Headless::with_fonts(fonts).map_err(native_error)?);
        }
        let comp = project.composition_id()?;
        let scene = Scene {
            doc: p.snapshot.document.clone(),
            comp,
            media: std::sync::Arc::new(media.export_paths(false)),
            content: p.revision(),
        };
        let rgba = self
            .headless
            .as_mut()
            .expect("initialized headless")
            .export_reference(
                &scene,
                ticks(request.time)?,
                Size::new(request.width, request.height),
            )
            .map_err(native_error)?;
        // PNG requires straight alpha. Reuse the exporter transform rather than
        // copying Chromium's premultiplied canvas readback quirk (N3).
        let mut bytes = Vec::with_capacity(rgba.len());
        for pixel in rgba.chunks_exact(4) {
            let (rgb, alpha) = pm_export::convert::output_rgba(
                [pixel[0], pixel[1], pixel[2], pixel[3]],
                pm_export::settings::WorkingEncoding::Standard,
                pm_export::settings::AlphaOutput::Straight,
            );
            bytes.extend(
                rgb.into_iter()
                    .chain([alpha])
                    .map(|v| (v.clamp(0., 1.) * 255.).round_ties_even() as u8),
            );
        }
        Ok(Image {
            png: png(request.width, request.height, &bytes)?,
            revision: p.revision(),
        })
    }
}
impl ExportChild for Native {
    fn export(&mut self, project: &Project, args: &Value, destination: &Path) -> Result<()> {
        let media = self.media(project)?;
        let p = project.published()?;
        let doc = project.document()?;
        let comp = project.composition_id()?;
        let format = match args["format"].as_str().unwrap_or("mp4") {
            "mp4" => ExportFormat::H264,
            "prores" => ExportFormat::ProRes {
                profile: if args["alpha"] == true {
                    ProResProfile::P4444
                } else {
                    ProResProfile::Hq
                },
            },
            _ => return Err(Error::invalid("Unsupported format")),
        };
        let rational = doc["fpsRational"]
            .as_array()
            .filter(|a| a.len() == 2)
            .and_then(|a| Some([a[0].as_i64()?, a[1].as_i64()?]));
        let rate = pm_time::rate_from_document(rational, doc["fps"].as_f64().unwrap_or(30.))
            .map_err(native_error)?
            .rate;
        let work = args["range"] == "work";
        let start = if work {
            doc["work"][0].as_f64().unwrap_or(0.)
        } else {
            0.
        };
        let end = if work {
            doc["work"][1].as_f64().or(doc["dur"].as_f64())
        } else {
            doc["dur"].as_f64()
        }
        .ok_or_else(|| Error::invalid("Missing duration"))?;
        let settings = ExportSettings {
            format,
            alpha: if args["alpha"] == true {
                AlphaOutput::Straight
            } else {
                AlphaOutput::None
            },
            quality: QualityTier::High,
            size: Size::new(
                doc["w"]
                    .as_u64()
                    .ok_or_else(|| Error::invalid("Invalid width"))? as u32,
                doc["h"]
                    .as_u64()
                    .ok_or_else(|| Error::invalid("Invalid height"))? as u32,
            ),
            frame_rate: FrameRate::new(rate.num() as u32, rate.den() as u32),
            range: TickRange {
                start: ticks(start)?,
                end: ticks(end)?,
            },
            audio: args["audio"].as_bool().unwrap_or(true),
            audio_index_playhead: None,
            motion_blur: args["motionBlur"].as_bool().unwrap_or(true),
            color: OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            },
            precision: Precision::Float16,
            dither: true,
            draft_from_proxies: false,
            delivery: Default::default(),
        };
        let request = ExportRequest::new(
            "headless-export".into(),
            &p.snapshot,
            JobSpec::Export {
                session: SessionId::new(1).expect("session id"),
                composition: comp.as_str().into(),
                settings,
            },
            media,
            destination.to_path_buf(),
        )
        .map_err(native_error)?;
        let status = ExportHandle::start(&std::env::current_exe()?, request)
            .map_err(native_error)?
            .wait()
            .map_err(native_error)?;
        if status.state != JobState::Done {
            return Err(error(
                "export_error",
                format!("Export child did not finish: {status:?}"),
            ));
        }
        Ok(())
    }
}
impl Backend for Native {}
