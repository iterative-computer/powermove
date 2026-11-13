//! Worker-only delivery planning and original-media project closure. No JS,
//! codec or filesystem work belongs on the UI thread.
use crate::LockShared;
use pm_kernel::Snapshot;
use pm_promedia::export_panel::{Format, Options};
use pm_protocol::{
    color::{OutputColor, Precision},
    common::{FrameRate, Size, TickRange, Ticks},
    priority::{AlphaOutput, ExportFormat, ExportSettings, QualityTier},
    validate::Validate,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// Kernel model numbers are f64; accept exactly integral dimensions without
/// rounding or truncating authored values during a settings round trip.
pub fn options(value: &Value) -> Result<Options, String> {
    let mut value = value.clone();
    for field in ["width", "height"] {
        if let Some(n) = value[field].as_f64()
            && n.is_finite()
            && n.fract() == 0.
            && n >= 0.
            && n <= u32::MAX as f64
        {
            value[field] = json!(n as u32);
        }
    }
    let options: Options = serde_json::from_value(value).map_err(|e| e.to_string())?;
    options.validate()?;
    Ok(options)
}
pub struct Plan {
    pub settings: Option<ExportSettings>,
    pub frames: u64,
    pub summary: String,
    pub captions: usize,
    pub has_audio: bool,
}
fn number(node: &pm_model::Node, key: &str) -> Result<f64, String> {
    node.get(key)
        .and_then(|n| match n.kind() {
            pm_model::NodeKind::Number(n) => Some(*n),
            _ => None,
        })
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("Missing composition {key}"))
}
/// Uses the engine's exact half-up frame-count rule on rational ticks. A work
/// range that is absent/malformed never silently becomes the whole composition.
pub fn plan(snapshot: &Snapshot, options: &Options, time: f64) -> Result<Plan, String> {
    options.validate()?;
    let root = snapshot.root().ok_or("Project closed")?;
    let comp = pm_kernel::model::comp_node(root, snapshot.open_comp().as_deref())
        .ok_or("Composition closed")?;
    let duration = number(comp, "dur")?;
    let layers = pm_kernel::model::list_layers(root, snapshot.open_comp().as_deref());
    let captions = layers.iter().filter(|l| l.kind == "captions").count();
    let has_audio = layers
        .iter()
        .any(|l| matches!(l.kind.as_str(), "audio" | "video" | "precomp"));
    if options.format == Format::Json {
        let name = root
            .get("name")
            .map(|n| n.to_value())
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        return Ok(Plan {
            settings: None,
            frames: 0,
            summary: format!("{name}.pmv"),
            captions,
            has_audio,
        });
    }
    let fps = if options.format == Format::Still {
        number(comp, "fps")?
    } else {
        options.fps
    };
    let rate = pm_time::rate_from_document(None, fps)
        .map_err(|e| e.to_string())?
        .rate;
    let (from, to) = if options.format == Format::Still {
        if !time.is_finite() || time < 0. || time > duration {
            return Err("Playhead is outside the composition".into());
        }
        (time, time + rate.den() as f64 / rate.num() as f64)
    } else if options.range == "work" {
        let work = comp
            .get("work")
            .map(|n| n.to_value())
            .ok_or("Work area is not set")?;
        let a = work
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("Invalid work area")?;
        (
            a[0].as_f64().ok_or("Invalid work area start")?,
            a[1].as_f64().ok_or("Invalid work area end")?,
        )
    } else {
        (0., duration)
    };
    if !from.is_finite()
        || !to.is_finite()
        || from < 0.
        || to <= from
        || (options.format != Format::Still && to > duration)
    {
        return Err("Invalid export range".into());
    }
    let start = pm_time::ticks_from_seconds(from).map_err(|e| e.to_string())?;
    let end = pm_time::ticks_from_seconds(to).map_err(|e| e.to_string())?;
    let unit = i128::from(rate.den()) * i128::from(pm_time::TICKS_PER_SECOND);
    let span = i128::from(end) - i128::from(start);
    let frames = if options.format == Format::Still {
        1
    } else {
        u64::try_from(((2 * span * i128::from(rate.num()) + unit) / (2 * unit)).max(1))
            .map_err(|_| "Frame count overflow")?
    };
    let (width, height) = match &options.scale {
        pm_promedia::export_panel::Scale::Preset(scale) => (
            (number(comp, "w")? * *scale).round(),
            (number(comp, "h")? * *scale).round(),
        ),
        pm_promedia::export_panel::Scale::Custom(_) => {
            (options.width as f64, options.height as f64)
        }
    };
    if !(16. ..=8192.).contains(&width) || !(16. ..=8192.).contains(&height) {
        return Err("Enter a size from 16 to 8192 px.".into());
    }
    let (width, height) = (width as u32, height as u32);
    let base = ExportSettings {
        format: ExportFormat::H264,
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: Size::new(width, height),
        frame_rate: FrameRate::new(
            u32::try_from(rate.num()).map_err(|_| "Frame rate overflow")?,
            u32::try_from(rate.den()).map_err(|_| "Frame rate overflow")?,
        ),
        range: TickRange {
            start: Ticks::new(start).map_err(|e| e.to_string())?,
            end: Ticks::new(end).map_err(|e| e.to_string())?,
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: if matches!(options.format, Format::Png | Format::Still) {
            OutputColor::SrgbImage
        } else {
            OutputColor::Rec709QuickTime {
                candidate: pm_protocol::color::QuickTimeTag::Cicp1_13_1,
            }
        },
        precision: Precision::Float16,
        dither: true,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    // A remembered HDR choice waits for an HDR project.
    let mut options = options.clone();
    let color = pm_render::composite::project_color(root)?;
    if color.mode != pm_protocol::color::ColorMode::Hdr {
        options.dynamic_range = pm_promedia::export_panel::DynamicRange::Sdr;
    }
    let settings = options
        .intent(has_audio, captions)?
        .engine_settings(base)?
        .ok_or("Missing render intent")?;
    settings.validate().map_err(|e| e.to_string())?;
    let summary = if options.format == Format::Still {
        let tc = pm_time::TimecodeFormat::for_rate(rate).format(rate.frame_of(start));
        format!("{}×{} · frame at {tc}", width, height)
    } else {
        // Two places at most, as the timeline's readouts: 73.53 s, 1.05 s, 10 s.
        let seconds = format!("{:.2}", pm_time::seconds_from_ticks(end - start));
        let seconds = seconds.trim_end_matches('0').trim_end_matches('.');
        format!("{frames} frames · {seconds} s · {}×{}", width, height)
    };
    Ok(Plan {
        settings: Some(settings),
        frames,
        summary,
        captions,
        has_audio,
    })
}
/// Retain all authored presets, including malformed legacy entries and unknown
/// fields. Same-name replacement marks predecessors archived; it never removes.
pub fn save_preset(stored: Option<Value>, name: &str, mut options: Value) -> Result<Value, String> {
    let typed = self::options(&options)?;
    typed.validate()?;
    // Main stores settings without the output name; never make a destination
    // part of a reusable preset. Existing records remain archived verbatim.
    if let Some(fields) = options.as_object_mut() {
        for field in ["name", "outputName", "destination"] {
            fields.remove(field);
        }
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("Preset name is empty".into());
    }
    let mut presets = match stored {
        None => vec![],
        Some(Value::Array(a)) => a,
        Some(_) => return Err("Saved presets are not a list".into()),
    };
    for p in &mut presets {
        if p["name"] == name
            && let Some(p) = p.as_object_mut()
        {
            p.insert("archived".into(), json!(true));
        }
    }
    presets.push(json!({"id":pm_kernel::new_id("preset"),"name":name,"options":options}));
    Ok(Value::Array(presets))
}
/// Export the current project into a separate PMV4 document; the source session,
/// history and dirty state remain untouched. Every persisted asset must resolve
/// to an original. Proxy maps are deliberately absent from this interface.
pub fn portable(
    snapshot: &Snapshot,
    originals: &BTreeMap<String, PathBuf>,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    use pm_kernel::{
        DocumentStore,
        project_io::{MediaEntry, MediaSource, SaveOptions, SaveTarget},
    };
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let root = snapshot.root().ok_or("Project closed")?;
    let mut store = DocumentStore::new_untitled(root.to_value());
    if let Some(assets) = root.get("assets")
        && let pm_model::NodeKind::Object(assets) = assets.kind()
    {
        for (id, record) in assets.iter() {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            let key = id.to_string_lossy();
            let path = originals
                .get(&key)
                .ok_or_else(|| format!("Original media is unavailable: {key}"))?;
            let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err(format!("Original media is not a file: {key}"));
            }
            let mime = record
                .get("mimeType")
                .or_else(|| record.get("mime"))
                .map(|n| n.to_value())
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "application/octet-stream".into());
            store.media_mut().insert(MediaEntry {
                id: id.clone(),
                media_type: mime.into(),
                revision: pm_kernel::new_id("media").into(),
                source: MediaSource::File {
                    path: path.clone(),
                    length: metadata.len(),
                    sha256: Some(
                        pm_kernel::project_io::hash_file(path).map_err(|e| e.to_string())?,
                    ),
                },
            });
        }
    }
    let plan = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: destination.into(),
                replace: false,
            },
            SaveOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let receipt = plan.run().map_err(|e| e.to_string())?;
    // Publication already happened. Cancellation after this boundary reports
    // completion and retains the verified file; it must never erase user output.
    Ok(receipt.path)
}

/// PMV publication needs original bytes, but no platform encoder or GPU.
pub fn portable_capability(snapshot: &Snapshot, originals: &BTreeMap<String, PathBuf>) -> Value {
    let check = (|| -> Result<(), String> {
        let root = snapshot.root().ok_or("Project closed")?;
        let Some(assets) = root.get("assets") else {
            return Ok(());
        };
        let pm_model::NodeKind::Object(assets) = assets.kind() else {
            return Err("Invalid media catalog".into());
        };
        // Count every unreadable original so the sheet can say how many to relink.
        let offline = assets
            .keys()
            .filter(|id| {
                !originals
                    .get(&id.to_string_lossy())
                    .and_then(|path| std::fs::metadata(path).ok())
                    .is_some_and(|m| m.is_file())
            })
            .count();
        match offline {
            0 => Ok(()),
            1 => Err("1 media file is offline. Relink it to save a portable project.".into()),
            n => Err(format!(
                "{n} media files are offline. Relink them to save a portable project."
            )),
        }
    })();
    match check {
        Ok(()) => json!({"available":true}),
        Err(reason) => json!({"available":false,"reason":reason}),
    }
}

/// Actual worker-side planner + encoder initialization, never an OS-name flag.
/// No frames are fabricated or delivered; codec-init failure remains unavailable.
#[cfg(target_os = "macos")]
pub fn capabilities(
    snapshot: &Snapshot,
    options: &Options,
    time: f64,
    originals: &BTreeMap<String, PathBuf>,
) -> Value {
    let mut result = json!({});
    for (name, format) in [
        ("prores", Format::Prores),
        ("mp4", Format::Mp4),
        ("webm", Format::Webm),
        ("png", Format::Png),
        ("still", Format::Still),
        ("json", Format::Json),
    ] {
        let check = (|| -> Result<(), String> {
            let mut options = options.clone();
            options.format = format;
            let planned = plan(snapshot, &options, time)?;
            if format == Format::Json {
                let root = snapshot.root().ok_or("Project closed")?;
                if let Some(assets) = root.get("assets")
                    && let pm_model::NodeKind::Object(assets) = assets.kind()
                {
                    for id in assets.keys() {
                        let path = originals
                            .get(&id.to_string_lossy())
                            .ok_or_else(|| format!("Original media is unavailable: {id}"))?;
                        std::fs::File::open(path).map_err(|e| e.to_string())?;
                    }
                }
                return Ok(());
            }
            let intent = options.intent(planned.has_audio, planned.captions)?;
            if intent.sidecar != "none" {
                return Err("Caption sidecar delivery is not connected".into());
            }
            let destination = tempfile::tempdir().map_err(|e| e.to_string())?;
            let job = pm_export::render::ExportJob {
                document: snapshot.document.clone(),
                composition: snapshot.open_comp().unwrap_or_default(),
                settings: planned.settings.ok_or("Missing render settings")?,
                media: std::sync::Arc::new(pm_export::render::MediaPaths {
                    paths: originals.clone(),
                    ..Default::default()
                }),
                destination: destination
                    .path()
                    .join(format!("probe.{}", format.extension())),
                overwrite: pm_export::Overwrite::Fail,
                mode: Some(0o600),
                seed: 0,
            };
            // Offline originals are the job's to name: it refuses with their
            // names and the sheet offers Export Without Them.
            let plan = pm_export::render::plan_export_with(&job, &probe_check)
                .map_err(|e| e.to_string())?;
            let _encoder = pm_export::OutputSink::new(&plan.spec).map_err(|e| e.to_string())?;
            Ok(())
        })();
        result[name] = match check {
            Ok(()) => json!({"available":true}),
            Err(reason) => unavailable(&reason),
        };
    }
    result
}

/// What the sheet can know before an export starts. The export job draws
/// text with the system's fonts and resolves effects, transitions and
/// extension layers against the built-in and installed definitions (a missing
/// one stops the job with its name and the choice to export without it), so
/// only the rest (media, image sequences, ...) can make a format unavailable.
#[cfg(target_os = "macos")]
fn probe_check(
    document: &pm_model::Document,
    composition: &str,
    media: &pm_export::render::MediaPaths,
) -> Vec<pm_render::Unsupported> {
    use pm_render::capability::Feature;
    pm_render::capability::unsupported_with_fonts(document, composition, Some(media))
        .into_iter()
        .filter(|u| {
            !matches!(
                u.feature,
                Feature::Effects
                    | Feature::Adjustment
                    | Feature::Transition
                    | Feature::ExtensionDefinition(_)
                    | Feature::Definition(_)
                    | Feature::EffectAsset(_)
            )
        })
        .collect()
}

/// An unavailable format: one short sentence for the sheet, the planner's full
/// list as `detail` (the footer's tooltip). The engine's unsupported list names
/// every layer and can run to pages.
pub fn unavailable(reason: &str) -> Value {
    const UNSUPPORTED: &str = "the new engine cannot render this yet: ";
    let Some(list) = reason.strip_prefix(UNSUPPORTED) else {
        return json!({"available":false,"reason":reason});
    };
    let items: Vec<&str> = list.split("; ").filter(|i| !i.trim().is_empty()).collect();
    let mut kinds: Vec<&str> = Vec::new();
    for item in &items {
        let kind = item.split([':', '(']).next().unwrap_or(item).trim();
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let short = format!(
        "{} {} in this project can’t be rendered natively yet ({}).",
        items.len(),
        if items.len() == 1 { "part" } else { "parts" },
        kinds.join(", ")
    );
    json!({"available":false,"reason":short,"detail":reason})
}

#[cfg(test)]
mod unavailable_tests {
    #[test]
    fn the_engines_list_becomes_one_sentence_with_the_list_as_detail() {
        let full = "the new engine cannot render this yet: effects (layer a in C); text: no font (layer b in C); effects (layer c in C)";
        let v = super::unavailable(full);
        assert_eq!(v["available"], false);
        assert_eq!(
            v["reason"],
            "3 parts in this project can’t be rendered natively yet (effects, text)."
        );
        assert_eq!(v["detail"], full);
        assert_eq!(super::unavailable("No encoder")["reason"], "No encoder");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::DocumentStore;
    fn fixture(extra: &str) -> Snapshot {
        let text = format!(
            r##"{{"id":"synthetic","name":"Film","w":640,"h":360,"fps":30,"dur":10,"layers":[{{"id":"caption","type":"captions","d":{{}}}}],"assets":{{}},"comps":{{}}{extra}}}"##
        );
        DocumentStore::new_untitled(pm_model::json::parse(&text).unwrap()).snapshot()
    }
    fn options() -> Options {
        serde_json::from_value(json!({"format":"mp4","scale":1,"width":640,"height":360,"fps":29.97,"range":"all","quality":"high","mblur":false,"audio":true,"alpha":false,"captionsBurn":true,"captionsSidecar":"none"})).unwrap()
    }
    #[test]
    fn the_planner_takes_color_tag_start_timecode_and_audio_format_from_the_sheet() {
        use pm_protocol::priority::AudioCoding;
        let asked = |extra: Value| {
            let mut v = json!({"format":"prores","profile":"422hq","scale":1,"width":640,"height":360,"fps":29.97,"range":"all","quality":"high","mblur":false,"audio":true,"alpha":false,"captionsBurn":false,"captionsSidecar":"none"});
            for (k, value) in extra.as_object().unwrap() {
                v[k] = value.clone();
            }
            plan(&fixture(""), &super::options(&v).unwrap(), 0.)
        };
        let s = asked(json!({"colorTag":"rec709","timecode":"01:00:00;00","audioCoding":"pcm16"}))
            .unwrap()
            .settings
            .unwrap();
        assert_eq!(s.color, pm_protocol::color::OutputColor::Rec709Broadcast);
        let timecode = s.delivery.timecode.unwrap();
        assert_eq!(
            (timecode.start.as_str(), timecode.drop_frame),
            ("01:00:00;00", true)
        );
        // The fixture has no sound, so there is no sound coding to choose.
        assert_eq!(s.delivery.audio_coding, None::<AudioCoding>);
        // Without the fields: today's file.
        let s = asked(json!({})).unwrap().settings.unwrap();
        assert!(matches!(
            s.color,
            pm_protocol::color::OutputColor::Rec709QuickTime { .. }
        ));
        assert_eq!(s.delivery.timecode, None);
        // A timecode the rate cannot count stops the plan with the sheet's words.
        let error = asked(json!({"timecode":"01:00:00:31"})).err().unwrap();
        assert!(error.contains("HH:MM:SS:FF"), "{error}");
    }
    #[test]
    fn hdr_delivery_follows_the_project_and_the_panel() {
        use pm_protocol::color::ColorMode;
        let mut o = options();
        o.dynamic_range = pm_promedia::export_panel::DynamicRange::Pq;
        // A remembered choice waits for an HDR project.
        let sdr = plan(&fixture(""), &o, 0.).unwrap().settings.unwrap();
        assert!(matches!(sdr.format, ExportFormat::H264));
        let hdr_project = fixture(r#","color":{"mode":"hdr"},"colorDepth":32"#);
        let root = hdr_project.root().unwrap();
        assert_eq!(
            pm_render::composite::project_color(root).unwrap().mode,
            ColorMode::Hdr
        );
        let hdr = plan(&hdr_project, &o, 0.).unwrap().settings.unwrap();
        assert_eq!(hdr.color, OutputColor::Rec2100Pq);
        assert!(matches!(
            hdr.format,
            ExportFormat::Hevc { ten_bit: true, .. }
        ));
        // Formats that hold no HDR leave the choice dormant.
        o.format = Format::Webm;
        let webm = plan(&hdr_project, &o, 0.).unwrap().settings.unwrap();
        assert_ne!(webm.color, OutputColor::Rec2100Pq);
        o.format = Format::Prores;
        let prores = plan(&hdr_project, &o, 0.).unwrap().settings.unwrap();
        assert_eq!(prores.color, OutputColor::Rec2100Pq);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn a_remembered_hdr_choice_leaves_every_format_available() {
        let mut o = options();
        o.dynamic_range = pm_promedia::export_panel::DynamicRange::Pq;
        for project in [
            fixture(""),
            fixture(r#","color":{"mode":"hdr"},"colorDepth":32"#),
        ] {
            let found = capabilities(&project, &o, 0., &BTreeMap::new());
            for format in ["prores", "mp4", "webm", "png", "still"] {
                assert_eq!(found[format]["available"], true, "{format}: {found}");
            }
        }
        // What `export.plan` runs on the panel's options.
        for format in [Format::Webm, Format::Png, Format::Still] {
            o.format = format;
            let wire = serde_json::to_value(&o).unwrap();
            assert_eq!(wire["dynamicRange"], "pq");
            assert_eq!(super::options(&wire).unwrap(), o);
        }
    }
    /// The sheet must not refuse what the export job draws: text (system
    /// fonts) and effects (definitions the job loads). A real project has both.
    #[cfg(target_os = "macos")]
    #[test]
    fn text_and_effects_leave_video_formats_available() {
        let project = DocumentStore::new_untitled(
            pm_model::json::parse(r##"{"id":"synthetic","compId":"C","name":"Film","w":640,"h":360,"fps":30,"dur":10,"assets":{},"comps":{},"layers":[
            {"id":"t","type":"text","from":0,"dur":10,"d":{"text":"Title"}},
            {"id":"s","type":"solid","from":0,"dur":10,"d":{"color":"#336699"},"fx":[{"type":"blur","p":{"radius":{"v":4}}}]}]}"##)
            .unwrap(),
        )
        .snapshot();
        let found = capabilities(&project, &options(), 0., &BTreeMap::new());
        for format in ["prores", "mp4", "webm", "png", "still"] {
            assert_eq!(found[format]["available"], true, "{format}: {found}");
        }
    }
    #[test]
    fn rational_work_range_and_frame_count_match_engine_half_up_rule() {
        let s = fixture(",\"work\":[2,3.05]");
        let mut o = options();
        o.range = "work".into();
        let p = plan(&s, &o, 0.).unwrap();
        let settings = p.settings.unwrap();
        assert_eq!(p.frames, 31);
        assert_eq!(settings.frame_rate, FrameRate::new(30000, 1001));
        assert_eq!(settings.range.start.get(), 2 * pm_time::TICKS_PER_SECOND);
        assert_eq!(p.captions, 1);
        assert!(!settings.delivery.hide_captions);
        assert!(p.summary.starts_with("31 frames · 1.05 s"));
        // 181 frames at 24 fps (7.5416… s) read at two places.
        o.fps = 24.;
        let s = fixture(",\"work\":[0,7.541666666666667]");
        assert!(plan(&s, &o, 0.).unwrap().summary.contains(" · 7.54 s · "));
    }
    #[test]
    fn missing_or_invalid_work_area_refuses_instead_of_exporting_all() {
        let mut o = options();
        o.range = "work".into();
        for extra in ["", ",\"work\":[4,2]", ",\"work\":[0,20]"] {
            assert!(plan(&fixture(extra), &o, 0.).is_err());
        }
    }
    #[test]
    fn still_is_one_frame_at_the_authored_playhead() {
        let mut o = options();
        o.format = Format::Still;
        let p = plan(&fixture(""), &o, 2.).unwrap();
        assert_eq!(p.frames, 1);
        assert_eq!(
            p.settings.unwrap().range.start.get(),
            2 * pm_time::TICKS_PER_SECOND
        );
        assert!(p.summary.contains("frame at "));
        assert!(plan(&fixture(""), &o, f64::NAN).is_err());
    }
    #[test]
    fn preset_replacement_archives_predecessors_and_preserves_opaque_data() {
        let mut o = serde_json::to_value(options()).unwrap();
        o["future"] = json!({"keep":[1,2,3]});
        let old = json!([{"id":"old","name":"Master","options":o,"extra":"keep"},17]);
        let next = save_preset(Some(old), " Master ", o.clone()).unwrap();
        assert_eq!(next[0]["extra"], "keep");
        assert_eq!(next[0]["archived"], true);
        assert_eq!(next[1], 17);
        assert_eq!(next[2]["options"], o);
        assert_ne!(next[2]["id"], "old");
        assert!(save_preset(Some(json!({"legacy":"retain"})), "Name", o).is_err());
    }
    #[test]
    fn portable_requires_original_closure_and_never_clobbers() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let source = dir.join("original.bin");
        std::fs::write(&source, b"synthetic original").unwrap();
        let mut root = fixture("").document.to_value().unwrap();
        root.as_object_mut().unwrap().insert("assets",pm_model::json::parse(r#"{"asset":{"name":"Original","type":"image","mimeType":"application/octet-stream"}}"#).unwrap());
        let s = DocumentStore::new_untitled(root).snapshot();
        let dest = dir.join("Film.pmv");
        assert!(portable(&s, &BTreeMap::new(), &dest, &AtomicBool::new(false)).is_err());
        assert!(!dest.exists());
        let originals = BTreeMap::from([("asset".into(), source.clone())]);
        portable(&s, &originals, &dest, &AtomicBool::new(false)).unwrap();
        let reopened = pm_kernel::project_io::open_project(&dest, Default::default()).unwrap();
        let copied = dir.join("copied.bin");
        reopened
            .file
            .media
            .get("asset")
            .unwrap()
            .copy_to(&copied)
            .unwrap();
        assert_eq!(std::fs::read(copied).unwrap(), b"synthetic original");
        let saved = std::fs::read(&dest).unwrap();
        assert!(portable(&s, &originals, &dest, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(dest).unwrap(), saved);
        assert_eq!(std::fs::read(source).unwrap(), b"synthetic original");
    }
    #[test]
    fn cancelled_portable_job_does_not_publish_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().canonicalize().unwrap().join("cancelled.pmv");
        assert!(
            portable(
                &fixture(""),
                &BTreeMap::new(),
                &dest,
                &AtomicBool::new(true)
            )
            .is_err()
        );
        assert!(!dest.exists());
    }
}

/// Path, kind, then length, modification time and inode.
type ProbeKey = (PathBuf, String, (u64, i64, i64, u64));

/// Decode results by file identity. Proving that media decodes costs a real
/// decode, so one file is probed once however many revisions observe it.
#[derive(Default)]
pub struct MediaProbes {
    results: Mutex<HashMap<ProbeKey, Result<(), String>>>,
    runs: AtomicU64,
}
impl MediaProbes {
    /// Real decodes run so far.
    pub fn runs(&self) -> u64 {
        self.runs.load(Ordering::Relaxed)
    }
    #[cfg(target_os = "macos")]
    fn result(&self, path: &Path, kind: &str) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let key = (
            path.to_path_buf(),
            kind.to_owned(),
            (
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ino(),
            ),
        );
        if let Some(known) = self.results.shared().get(&key) {
            return known.clone();
        }
        self.runs.fetch_add(1, Ordering::Relaxed);
        let result = probe_media(path, kind);
        let mut results = self.results.shared();
        if results.len() >= 16_384 {
            results.clear();
        }
        results.insert(key, result.clone());
        result
    }
}
pub(crate) fn probe_media(path: &Path, kind: &str) -> Result<(), String> {
    match kind {
        "image" => pm_render::media::probe_image(path).map_err(|issue| match issue {
            pm_render::media::ProbeIssue::Unreadable(message) => message,
            pm_render::media::ProbeIssue::Unsupported(feature) => feature.to_string(),
        }),
        #[cfg(not(target_os = "macos"))]
        "video" => Err("Video decoding needs macOS".into()),
        #[cfg(target_os = "macos")]
        "video" => pm_render::media::probe_video(path).and_then(|_| {
            use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
            let service = DecodeService::new(ServiceConfig {
                cache_bytes: 0,
                pool_free_bytes: 0,
                threads: 1,
                idle_sessions: 0,
                session_window: 1,
            });
            let source = MediaSource::open(path).map_err(|e| e.to_string())?;
            let mut decoder = service
                .session(&source, 0, pm_media::vt::DecodePurpose::Display)
                .map_err(|e| e.to_string())?;
            // Indexing alone cannot prove that an original decodes.
            decoder.frame(0).map(|_| ()).map_err(|e| e.to_string())
        }),
        _ => {
            let cache = pm_audio::SourceCache::new(1024 * 1024, 1);
            cache
                .open(path, None)
                .and_then(|source| cache.block(&source, 0, true).map(|_| ()))
                .map_err(|e| e.to_string())
        }
    }
}
/// Native decoder probes; no metadata-only Ready state or proxy promotion.
#[cfg(target_os = "macos")]
pub fn observe_media(
    project: &str,
    snapshot: &Snapshot,
    originals: &BTreeMap<String, PathBuf>,
    proxies: &BTreeMap<String, PathBuf>,
    probes: &MediaProbes,
) -> Result<pm_panel_media::readiness::ObservedRuntime, String> {
    use pm_panel_media::{
        Availability, Catalog, ProxyStatus, RuntimeState, readiness::ObservedRuntime,
    };
    let catalog = Catalog::new(project.into(), snapshot.clone())
        .map_err(|_| "The project's media could not be read")?;
    let mut states = BTreeMap::new();
    for (id, asset) in &catalog.assets {
        let availability = match originals.get(id) {
            None => Availability::Offline,
            Some(path) => {
                if !matches!(asset.kind.as_str(), "image" | "video" | "audio") {
                    states.insert(id.clone(), RuntimeState::default());
                    continue;
                }
                match probes.result(path, &asset.kind) {
                    Ok(()) => Availability::Ready,
                    Err(e) => Availability::Failed(e),
                }
            }
        };
        states.insert(
            id.clone(),
            RuntimeState {
                availability,
                proxy: if proxies.contains_key(id) {
                    ProxyStatus::Unknown
                } else {
                    ProxyStatus::Absent
                },
                ..Default::default()
            },
        );
    }
    Ok(ObservedRuntime::new(catalog.stamp, states))
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    #[test]
    fn a_revision_never_decodes_a_file_again_until_that_file_changes() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("clip.png");
        std::fs::write(&file, b"not an image").unwrap();
        let project = pm_model::json::parse(
            r##"{"id":"p","name":"Synthetic","compId":"c","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000","assets":{"a":{"name":"a.png","kind":"image"}},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        let snapshot = pm_kernel::DocumentStore::new_untitled(project).snapshot();
        let originals = BTreeMap::from([("a".to_owned(), file.clone())]);
        let probes = MediaProbes::default();
        for _ in 0..25 {
            observe_media("p", &snapshot, &originals, &BTreeMap::new(), &probes).unwrap();
        }
        assert_eq!(probes.runs(), 1);
        std::fs::write(&file, b"replaced bytes").unwrap();
        observe_media("p", &snapshot, &originals, &BTreeMap::new(), &probes).unwrap();
        assert_eq!(probes.runs(), 2, "a changed file is decoded again");
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    #[test]
    fn preset_size_uses_composition_and_custom_size_preserves_authored_dimensions() {
        let s=pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(r#"{"id":"p","name":"Film","w":640,"h":360,"fps":30,"dur":1,"layers":[],"assets":{}}"#).unwrap()).snapshot();
        let mut options=options(&json!({"format":"mp4","scale":0.5,"width":1000.0,"height":600.0,"fps":24,"range":"all","quality":"high","mblur":false,"audio":false,"alpha":false,"captionsBurn":false,"captionsSidecar":"none"})).unwrap();
        let half = plan(&s, &options, 0.).unwrap().settings.unwrap();
        assert_eq!(half.size, Size::new(320, 180));
        options.scale = pm_promedia::export_panel::Scale::Custom("custom".into());
        let custom = plan(&s, &options, 0.).unwrap().settings.unwrap();
        assert_eq!(custom.size, Size::new(1000, 600));
        options.format = Format::Still;
        assert_eq!(
            plan(&s, &options, 0.5)
                .unwrap()
                .settings
                .unwrap()
                .frame_rate,
            FrameRate::new(30, 1)
        );
    }
    #[test]
    fn fractional_dimensions_are_never_truncated_during_a_saved_settings_roundtrip() {
        let invalid = json!({"format":"mp4","scale":1,"width":640.25,"height":360,"fps":30,"range":"all","quality":"high","mblur":false,"audio":false,"alpha":false,"captionsBurn":false,"captionsSidecar":"none"});
        assert!(options(&invalid).is_err());
    }
}

/// Migration provenance markers describe the stored bytes, independently of
/// authored dimensions/fingerprints. Never promote an embedded proxy to original.
pub fn embedded_proxy(snapshot: &Snapshot, asset: &str) -> bool {
    let record = snapshot
        .root()
        .and_then(|r| r.get("assets"))
        .and_then(|a| a.get(asset));
    record.is_some_and(|r| {
        r.get("playbackProxy")
            .is_some_and(|n| matches!(n.kind(), pm_model::NodeKind::Bool(true)))
            || r.get("playbackProxyVersion")
                .is_some_and(|n| matches!(n.kind(),pm_model::NodeKind::Number(v) if *v!=0.))
    })
}
#[cfg(test)]
mod provenance_tests {
    use super::*;
    #[test]
    fn either_migration_proxy_marker_prevents_original_promotion() {
        let snapshot=pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(r#"{"assets":{"a":{"playbackProxy":true},"b":{"playbackProxyVersion":2},"c":{"playbackProxy":false,"playbackProxyVersion":0},"d":{"fingerprint":"synthetic"}}}"#).unwrap()).snapshot();
        assert!(embedded_proxy(&snapshot, "a"));
        assert!(embedded_proxy(&snapshot, "b"));
        assert!(!embedded_proxy(&snapshot, "c"));
        assert!(!embedded_proxy(&snapshot, "d"));
        assert!(!embedded_proxy(&snapshot, "missing"));
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn pmv_is_platform_independent_but_requires_every_original_to_be_a_readable_file() {
        let snapshot = pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(r#"{"assets":{"asset":{"type":"image"}}}"#).unwrap(),
        )
        .snapshot();
        let dir = tempfile::tempdir().unwrap();
        let mut originals = BTreeMap::new();
        assert_eq!(
            portable_capability(&snapshot, &originals)["available"],
            false
        );
        originals.insert("asset".into(), dir.path().to_owned());
        assert_eq!(
            portable_capability(&snapshot, &originals)["available"],
            false
        );
        let path = dir.path().join("source.bin");
        std::fs::write(&path, b"synthetic original").unwrap();
        originals.insert("asset".into(), path);
        assert_eq!(
            portable_capability(&snapshot, &originals)["available"],
            true
        );
    }
}
