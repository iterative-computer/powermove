use pm_promedia::export_panel::*;
use serde_json::json;
fn options() -> Options {
    serde_json::from_value(json!({"format":"mp4","scale":1,"width":1920,"height":1080,
        "fps":30000./1001.,"range":"work","quality":"high","mblur":true,"audio":true,
        "alpha":true,"captionsBurn":true,"captionsSidecar":"srt","future":{"keep":true}}))
    .unwrap()
}
#[derive(Default)]
struct Host {
    calls: Vec<String>,
    destination: Option<Destination>,
    unavailable: bool,
    fail_remember: bool,
    fail_render: bool,
    intent: Option<Intent>,
}
impl ExportHost for Host {
    type Frozen = u64;
    type Job = u64;
    fn available(&self, _: Format) -> Result<(), String> {
        if self.unavailable {
            Err("unavailable".into())
        } else {
            Ok(())
        }
    }
    fn remember(&mut self, _: &Options) -> Result<u64, String> {
        self.calls.push("remember".into());
        if self.fail_remember {
            Err("disk write failed".into())
        } else {
            Ok(73)
        }
    }
    fn choose_destination(
        &mut self,
        suggested: &str,
        folder: bool,
    ) -> Result<Option<Destination>, String> {
        self.calls.push(format!("choose:{suggested}:{folder}"));
        Ok(self.destination.clone())
    }
    fn render(&mut self, frozen: u64, intent: Intent, _: Destination) -> Result<u64, String> {
        assert_eq!(frozen, 73);
        self.calls.push("render".into());
        self.intent = Some(intent);
        if self.fail_render {
            Err("engine refused".into())
        } else {
            Ok(9)
        }
    }
    fn save_project(&mut self, frozen: u64, _: std::path::PathBuf) -> Result<u64, String> {
        assert_eq!(frozen, 73);
        self.calls.push("save_project".into());
        Ok(10)
    }
}
#[test]
fn destination_precedes_any_engine_work_and_cancel_keeps_defaults() {
    let mut h = Host::default();
    assert_eq!(
        begin(&mut h, "Film", &options(), true, 1).unwrap(),
        BeginResult::DestinationCancelled
    );
    assert_eq!(h.calls, ["remember", "choose:Film.mp4:false"]);
    h.destination = Some(Destination::File("chosen.mp4".into()));
    assert_eq!(
        begin(&mut h, "Film", &options(), true, 1).unwrap(),
        BeginResult::Started(9)
    );
    assert_eq!(h.calls.last().unwrap(), "render");
}
#[test]
fn png_chooses_folder_and_project_uses_portable_writer() {
    let mut h = Host {
        destination: Some(Destination::Folder("frames".into())),
        ..Host::default()
    };
    let mut o = options();
    o.format = Format::Png;
    begin(&mut h, "Film", &o, true, 1).unwrap();
    assert_eq!(h.calls[1], "choose:Film.png:true");
    o.format = Format::Json;
    h.destination = Some(Destination::File("film.pmv".into()));
    assert_eq!(
        begin(&mut h, "Film", &o, true, 1).unwrap(),
        BeginResult::Started(10)
    );
    assert_eq!(h.calls.last().unwrap(), "save_project");
}
#[test]
fn unsupported_format_and_failed_persistence_never_open_picker() {
    let mut h = Host {
        unavailable: true,
        ..Host::default()
    };
    assert!(begin(&mut h, "Film", &options(), true, 1).is_err());
    assert!(h.calls.is_empty());
    h.unavailable = false;
    h.fail_remember = true;
    assert!(begin(&mut h, "Film", &options(), true, 1).is_err());
    assert_eq!(h.calls, ["remember"]);
}
#[test]
fn engine_errors_and_wrong_destination_are_not_success() {
    let mut h = Host {
        destination: Some(Destination::Folder("wrong".into())),
        ..Host::default()
    };
    assert!(begin(&mut h, "Film", &options(), true, 1).is_err());
    assert!(!h.calls.iter().any(|s| s == "render"));
    h.destination = Some(Destination::File("right".into()));
    h.fail_render = true;
    assert_eq!(
        begin(&mut h, "Film", &options(), true, 1),
        Err("engine refused".into())
    );
}
#[test]
fn effective_fields_match_spec_without_erasing_dormant_authored_intent() {
    for format in [
        Format::Prores,
        Format::Mp4,
        Format::Webm,
        Format::Png,
        Format::Still,
        Format::Json,
    ] {
        let mut o = options();
        o.format = format;
        let i = o.intent(true, 2).unwrap();
        assert_eq!(
            i.audio,
            matches!(format, Format::Prores | Format::Mp4 | Format::Webm)
        );
        assert_eq!(
            i.straight_alpha,
            matches!(format, Format::Prores | Format::Png | Format::Still)
        );
        assert_eq!(i.burn_captions, format != Format::Json);
        assert_eq!(i.options, o);
    }
    let i = options().intent(false, 0).unwrap();
    assert!(!i.audio);
    assert!(!i.burn_captions);
    assert_eq!(i.sidecar, "none");
}
#[test]
fn invalid_sizes_scale_and_values_are_refused_before_side_effects() {
    for size in [0, 15, 8193, u32::MAX] {
        let mut o = options();
        o.width = size;
        assert!(o.validate().is_err());
    }
    let mut o = options();
    o.scale = Scale::Custom("custom".into());
    assert!(o.validate().is_ok());
    o.scale = Scale::Preset(f64::NAN);
    assert!(o.validate().is_err());
    o = options();
    o.fps = f64::INFINITY;
    assert!(o.validate().is_err());
    o = options();
    o.captions_sidecar = "invalid".into();
    assert!(o.validate().is_err());
}
#[test]
fn wire_roundtrip_preserves_unknown_settings_and_exact_rate() {
    let o = options();
    let round: Options = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
    assert_eq!(round, o);
    assert_eq!(round.fps, 30000. / 1001.);
    assert_eq!(round.opaque["future"], json!({"keep":true}));
}
#[test]
fn web_export_is_not_admitted() {
    let mut raw = serde_json::to_value(options()).unwrap();
    raw["format"] = json!("web");
    assert!(serde_json::from_value::<Options>(raw).is_err());
}

#[test]
fn portable_project_does_not_validate_hidden_raster_fields() {
    let mut o = options();
    o.format = Format::Json;
    o.width = 12000;
    o.height = 2;
    assert!(o.validate().is_ok());
    let mut h = Host {
        destination: Some(Destination::File("wide.pmv".into())),
        ..Host::default()
    };
    assert_eq!(
        begin(&mut h, "Wide", &o, false, 0).unwrap(),
        BeginResult::Started(10)
    );
}

fn planned() -> pm_protocol::priority::ExportSettings {
    use pm_protocol::{
        color::{OutputColor, Precision},
        common::{FrameRate, Size, TickRange, Ticks},
        priority::*,
    };
    ExportSettings {
        format: ExportFormat::H264,
        alpha: AlphaOutput::None,
        quality: QualityTier::Draft,
        size: Size::new(1280, 720),
        frame_rate: FrameRate::new(24, 1),
        range: TickRange {
            start: Ticks::from_whole_seconds(2),
            end: Ticks::from_whole_seconds(10),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::Rec709Broadcast,
        precision: Precision::Float32,
        dither: true,
        draft_from_proxies: true,
        delivery: Default::default(),
    }
}
#[test]
fn engine_mapping_preserves_native_plan_and_exact_rate_without_proxy_fallback() {
    use pm_protocol::priority::*;
    let i = options().intent(true, 1).unwrap();
    let base = planned();
    let settings = i.engine_settings(base.clone()).unwrap().unwrap();
    assert_eq!(settings.format, ExportFormat::H264);
    assert_eq!(
        settings.frame_rate,
        pm_protocol::common::FrameRate::new(30000, 1001)
    );
    assert_eq!(settings.range, base.range);
    assert_eq!(settings.size, base.size);
    assert_eq!(settings.color, base.color);
    assert_eq!(settings.precision, base.precision);
    assert_eq!(settings.alpha, AlphaOutput::None);
    assert!(!settings.draft_from_proxies);
    assert!(settings.audio);
    assert!(settings.delivery.preview);
    assert!(!settings.delivery.hide_captions);
    assert_eq!(i.sidecar, "srt");
}
#[test]
fn every_render_format_maps_to_real_engine_contract_and_pmv_does_not_render() {
    use pm_protocol::{color::OutputColor, priority::*};
    for format in [
        Format::Prores,
        Format::Mp4,
        Format::Webm,
        Format::Png,
        Format::Still,
        Format::Json,
    ] {
        let mut o = options();
        o.format = format;
        let i = o.intent(true, 0).unwrap();
        let mut base = planned();
        if matches!(format, Format::Png | Format::Still) {
            base.color = OutputColor::SrgbImage;
        }
        let mapped = i.engine_settings(base.clone()).unwrap();
        if format == Format::Json {
            assert!(mapped.is_none());
            continue;
        }
        let mapped = mapped.unwrap();
        if format == Format::Prores {
            assert_eq!(
                mapped.format,
                ExportFormat::ProRes {
                    profile: ProResProfile::P4444
                }
            );
            assert_eq!(mapped.alpha, AlphaOutput::Straight);
        }
        if format == Format::Webm {
            assert_eq!(
                mapped.format,
                ExportFormat::Webm {
                    codec: WebmCodec::Vp9
                }
            );
            assert_eq!(mapped.alpha, AlphaOutput::None);
        }
        if format == Format::Still {
            assert_eq!(mapped.frame_rate, base.frame_rate);
        }
        assert!(mapped.delivery.hide_captions);
    }
}

#[test]
fn hdr_delivery_picks_a_ten_bit_codec_and_bt2100_tags() {
    use pm_promedia::export_panel::DynamicRange;
    use pm_protocol::color::OutputColor;
    use pm_protocol::priority::{Container, ExportFormat};
    let mut options: Options = serde_json::from_value(serde_json::json!({
        "format":"mp4","scale":1,"width":640,"height":360,"fps":30,"range":"all",
        "quality":"high","mblur":false,"audio":false,"alpha":false,
        "captionsBurn":false,"captionsSidecar":"none","dynamicRange":"pq"
    }))
    .unwrap();
    assert_eq!(options.dynamic_range, DynamicRange::Pq);
    let saved = serde_json::to_value(&options).unwrap();
    assert_eq!(saved["dynamicRange"], "pq");
    let planned = hdr_base();
    let settings = options
        .intent(false, 0)
        .unwrap()
        .engine_settings(planned.clone())
        .unwrap()
        .unwrap();
    assert_eq!(settings.color, OutputColor::Rec2100Pq);
    assert!(matches!(
        settings.format,
        ExportFormat::Hevc {
            ten_bit: true,
            container: Container::Mp4
        }
    ));
    options.format = Format::Prores;
    options.dynamic_range = DynamicRange::Hlg;
    let settings = options
        .intent(false, 0)
        .unwrap()
        .engine_settings(planned)
        .unwrap()
        .unwrap();
    assert_eq!(settings.color, OutputColor::Rec2100Hlg);
    options.format = Format::Webm;
    assert!(options.validate().is_err());
    options.dynamic_range = DynamicRange::Sdr;
    assert!(options.validate().is_ok());
    assert!(
        serde_json::to_value(&options)
            .unwrap()
            .get("dynamicRange")
            .is_none()
    );
}

fn hdr_base() -> pm_protocol::priority::ExportSettings {
    use pm_protocol::common::{FrameRate, Size, TickRange, Ticks};
    use pm_protocol::priority::*;
    ExportSettings {
        format: ExportFormat::H264,
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: Size::new(640, 360),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::from_whole_seconds(1),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: pm_protocol::color::OutputColor::Rec709QuickTime {
            candidate: pm_protocol::color::QuickTimeTag::Cicp1_13_1,
        },
        precision: pm_protocol::color::Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}
