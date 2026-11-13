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
fn every_profile_codec_and_depth_reaches_the_engine_and_old_settings_mean_today() {
    use pm_protocol::priority::*;
    let old = options();
    assert_eq!(
        (old.profile, old.codec, old.depth),
        (Profile::P4444, Codec::H264, 8)
    );
    let map = |patch: serde_json::Value| {
        let mut value = serde_json::to_value(options()).unwrap();
        for (k, v) in patch.as_object().unwrap() {
            value[k] = v.clone();
        }
        let o: Options = serde_json::from_value(value).unwrap();
        let i = o.intent(true, 0).unwrap();
        let mut base = planned();
        if matches!(o.format, Format::Png | Format::Still) {
            base.color = pm_protocol::color::OutputColor::SrgbImage;
        }
        let mapped = i.engine_settings(base).unwrap().unwrap();
        (mapped.format, mapped.alpha)
    };
    for (id, profile, alpha) in [
        ("422proxy", ProResProfile::Proxy, AlphaOutput::None),
        ("422lt", ProResProfile::Lt, AlphaOutput::None),
        ("422", ProResProfile::Standard, AlphaOutput::None),
        ("422hq", ProResProfile::Hq, AlphaOutput::None),
        ("4444", ProResProfile::P4444, AlphaOutput::Straight),
        ("4444xq", ProResProfile::P4444Xq, AlphaOutput::Straight),
    ] {
        assert_eq!(
            map(json!({"format":"prores","profile":id})),
            (ExportFormat::ProRes { profile }, alpha),
            "{id}"
        );
    }
    assert_eq!(
        map(json!({"codec":"hevc"})).0,
        ExportFormat::Hevc {
            ten_bit: true,
            container: Container::Mp4
        }
    );
    assert_eq!(map(json!({"codec":"h264"})).0, ExportFormat::H264);
    assert_eq!(
        map(json!({"format":"png","depth":16})).0,
        ExportFormat::PngSequence { sixteen_bit: true }
    );
    assert_eq!(
        map(json!({"format":"still","depth":16})).0,
        ExportFormat::PngStill { sixteen_bit: true }
    );
    let mut bad = serde_json::to_value(options()).unwrap();
    bad["depth"] = json!(10);
    let bad: Options = serde_json::from_value(bad).unwrap();
    assert!(bad.validate().is_err());
    let mut unknown = serde_json::to_value(options()).unwrap();
    unknown["profile"] = json!("dnxhr");
    assert!(serde_json::from_value::<Options>(unknown).is_err());
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
        .engine_settings(planned.clone())
        .unwrap()
        .unwrap();
    assert_eq!(settings.color, OutputColor::Rec2100Hlg);
    // A format without HDR keeps the choice dormant and delivers SDR.
    options.format = Format::Webm;
    assert!(options.validate().is_ok());
    let settings = options
        .intent(false, 0)
        .unwrap()
        .engine_settings(planned)
        .unwrap()
        .unwrap();
    assert!(!matches!(
        settings.color,
        OutputColor::Rec2100Pq | OutputColor::Rec2100Hlg
    ));
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

/// Use Proxies (Draft) applies only where the sheet shows it: a still
/// export that inherited it from a video setting reads originals.
#[test]
fn a_hidden_draft_choice_never_reaches_the_engine() {
    for (format, draft) in [
        (Format::Prores, true),
        (Format::Mp4, true),
        (Format::Webm, true),
        (Format::Png, true),
        (Format::Still, false),
    ] {
        let mut o = options();
        o.format = format;
        o.use_proxies = true;
        let mut base = planned();
        base.draft_from_proxies = false;
        if matches!(format, Format::Png | Format::Still) {
            base.color = pm_protocol::color::OutputColor::SrgbImage;
        }
        let mapped = o
            .intent(true, 0)
            .unwrap()
            .engine_settings(base)
            .unwrap()
            .unwrap();
        assert_eq!(mapped.draft_from_proxies, draft, "{format:?}");
    }
    let mut o = options();
    o.use_proxies = false;
    let mapped = o
        .intent(true, 0)
        .unwrap()
        .engine_settings(planned())
        .unwrap()
        .unwrap();
    assert!(!mapped.draft_from_proxies, "originals unless chosen");
}
#[test]
fn export_defaults_normalize_exactly_as_main() {
    use pm_promedia::export_panel::normalize_defaults as n;
    let d = n(&json!(null), 25.);
    assert_eq!(
        d,
        json!({"format":"mp4","scale":1.0,"fps":25.0,"range":"work","quality":"high","mblur":true,"audio":true,"alpha":false})
    );
    // Number() coercion and clamps, kept v1 formats, booleans only as booleans.
    let d = n(
        &json!({"format":"web","scale":"40","fps":"300","range":"all","quality":"max","mblur":"no","audio":false,"alpha":1}),
        30.,
    );
    assert_eq!(d["format"], "web");
    assert_eq!(d["scale"], 16.0);
    assert_eq!(d["fps"], 240.0);
    assert_eq!(d["range"], "all");
    assert_eq!(d["mblur"], true);
    assert_eq!(d["audio"], false);
    assert_eq!(d["alpha"], false);
    assert_eq!(
        n(&json!({"scale":-1,"fps":0}), f64::NAN),
        n(&json!({}), 30.)
    );
    assert_eq!(n(&json!({"scale":0.001}), 30.)["scale"], 0.01);
}

fn prores(extra: serde_json::Value) -> Options {
    let mut v = json!({"format":"prores","profile":"422hq","scale":1,"width":1920,"height":1080,
        "fps":25,"range":"all","quality":"high","mblur":true,"audio":true,
        "alpha":false,"captionsBurn":false,"captionsSidecar":"none"});
    for (k, value) in extra.as_object().unwrap() {
        v[k] = value.clone();
    }
    serde_json::from_value(v).unwrap()
}

#[test]
fn colour_label_start_timecode_and_sound_coding_reach_the_engine() {
    use pm_protocol::color::{OutputColor, QuickTimeTag};
    use pm_protocol::priority::{AudioCoding as Wire, ExportTimecode};
    let mut base = planned();
    base.color = OutputColor::Rec709QuickTime {
        candidate: QuickTimeTag::Cicp1_13_1,
    };
    let settings = |o: &Options| {
        o.intent(true, 0)
            .unwrap()
            .engine_settings(base.clone())
            .unwrap()
            .unwrap()
    };
    // Settings saved before these fields existed mean today's file.
    let old = settings(&prores(json!({})));
    assert_eq!(old.color, base.color);
    assert_eq!(old.delivery.timecode, None);
    assert_eq!(old.delivery.audio_coding, None);
    // What a post house asks for.
    let asked = prores(json!({"colorTag":"rec709","timecode":"01:00:00:00","audioCoding":"pcm16"}));
    let s = settings(&asked);
    assert_eq!(s.color, OutputColor::Rec709Broadcast);
    assert_eq!(
        s.delivery.timecode,
        Some(ExportTimecode {
            start: "01:00:00:00".into(),
            drop_frame: false,
            reel: None
        })
    );
    assert_eq!(s.delivery.audio_coding, Some(Wire::Pcm16));
    assert_eq!(
        settings(&prores(json!({"audioCoding":"pcmFloat"})))
            .delivery
            .audio_coding,
        Some(Wire::PcmFloat)
    );
    // Drop-frame is written with a semicolon, at the rates that have it.
    let ntsc = prores(json!({"fps":30000./1001.,"timecode":"01:00:00;00"}));
    assert!(settings(&ntsc).delivery.timecode.unwrap().drop_frame);
    // Sound coding is the sound's: nothing without it.
    let silent = prores(json!({"audio":false,"audioCoding":"pcm16"}));
    assert_eq!(settings(&silent).delivery.audio_coding, None);
    // The fields stay stored and dormant where a format does not read them:
    // MP4 has the label but no timecode track or PCM; WebM and PNG have none.
    let mut mp4 = asked.clone();
    mp4.format = Format::Mp4;
    let s = settings(&mp4);
    assert_eq!(s.color, OutputColor::Rec709Broadcast);
    assert_eq!((s.delivery.timecode, s.delivery.audio_coding), (None, None));
    for format in [Format::Webm, Format::Png, Format::Still] {
        let mut other = asked.clone();
        other.format = format;
        // The planner gives images an image tag; the label is not theirs.
        let mut planned = base.clone();
        if format != Format::Webm {
            planned.color = OutputColor::SrgbImage;
        }
        let s = other
            .intent(true, 0)
            .unwrap()
            .engine_settings(planned.clone())
            .unwrap()
            .unwrap();
        assert_eq!(s.color, planned.color, "{format:?}");
        assert_eq!((s.delivery.timecode, s.delivery.audio_coding), (None, None));
    }
    // HDR delivery carries its own label.
    let hdr = prores(json!({"colorTag":"rec709","dynamicRange":"pq"}));
    assert_eq!(settings(&hdr).color, OutputColor::Rec2100Pq);
    // Round trip: absent stays absent, chosen stays chosen.
    let wire = serde_json::to_value(prores(json!({}))).unwrap();
    for key in ["colorTag", "timecode", "audioCoding"] {
        assert!(wire.get(key).is_none(), "{key} written for old settings");
    }
    let wire = serde_json::to_value(&asked).unwrap();
    assert_eq!(
        (&wire["colorTag"], &wire["timecode"], &wire["audioCoding"]),
        (&json!("rec709"), &json!("01:00:00:00"), &json!("pcm16"))
    );
}

#[test]
fn a_start_timecode_the_rate_cannot_count_is_refused_in_the_sheet() {
    for (extra, why) in [
        (json!({"timecode":"1:00:00:00"}), "HH:MM:SS:FF"),
        (json!({"timecode":"01:00:00:25"}), "HH:MM:SS:FF"),
        (json!({"timecode":"01:61:00:00"}), "HH:MM:SS:FF"),
        (json!({"timecode":"01:00:00;00"}), "29.97 or 59.94"),
        (
            json!({"fps":30000./1001.,"timecode":"01:01:00;00"}),
            "HH:MM:SS:FF",
        ),
    ] {
        let error = prores(extra.clone()).intent(true, 0).unwrap_err();
        assert!(error.contains(why), "{extra}: {error}");
    }
    // The same text in a format without a timecode track is dormant, not an error.
    let mut mp4 = prores(json!({"timecode":"nonsense"}));
    mp4.format = Format::Mp4;
    assert!(mp4.intent(true, 0).is_ok());
}
