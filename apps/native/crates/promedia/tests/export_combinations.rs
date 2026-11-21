use pm_export::{self as engine, *};
use pm_promedia::export_panel::{self as sheet, *};
use pm_protocol::{color::OutputColor, common::*, priority as wire};
use serde_json::json;
use std::collections::BTreeSet;

fn planned(image: bool) -> wire::ExportSettings {
    wire::ExportSettings {
        format: wire::ExportFormat::H264,
        alpha: wire::AlphaOutput::None,
        quality: wire::QualityTier::High,
        size: Size::new(640, 360),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::from_whole_seconds(1),
        },
        audio: true,
        audio_index_playhead: None,
        motion_blur: false,
        color: if image {
            OutputColor::SrgbImage
        } else {
            OutputColor::Rec709Broadcast
        },
        precision: pm_protocol::color::Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

fn spec(codec: engine::Codec, container: engine::Container) -> ExportSpec {
    ExportSpec {
        width: 640,
        height: 360,
        rate: pm_time::Rate::FPS_30,
        frames: 1,
        codec,
        container,
        tags: if matches!(codec, engine::Codec::Png { .. }) {
            ColorTags::SrgbImage
        } else {
            ColorTags::Rec709Broadcast
        },
        alpha: engine::AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: "output".into(),
        overwrite: Overwrite::Fail,
        mode: None,
    }
}

fn from_sheet(o: &Options) -> Result<ExportSpec, String> {
    let s = o
        .intent(true, 0)?
        .engine_settings(planned(matches!(o.format, Format::Png | Format::Still)))?
        .ok_or("project")?;
    let (codec, container) = engine::wire::codec_of(&s);
    let mut spec = spec(codec, container);
    spec.alpha = match s.alpha {
        wire::AlphaOutput::None => engine::AlphaOutput::None,
        wire::AlphaOutput::Straight => engine::AlphaOutput::Straight,
        wire::AlphaOutput::Premultiplied => engine::AlphaOutput::Premultiplied,
    };
    if s.audio {
        spec.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: engine::wire::audio_codec_of(&s, container),
        });
    }
    if container == engine::Container::Mov && !s.delivery.without_timecode {
        spec.timecode = Some(TimecodeSpec {
            start_frame: 0,
            format: pm_time::TimecodeFormat::non_drop(spec.rate),
            reel: None,
        });
    }
    Ok(spec)
}

fn tuple(s: &ExportSpec) -> String {
    format!(
        "{:?}/{:?}/{:?}/{:?}/{}",
        s.container,
        s.codec,
        s.alpha,
        s.audio.map(|a| a.codec),
        s.timecode.is_some()
    )
}

#[test]
fn every_accepted_delivery_tuple_is_reachable_and_every_sheet_selection_validates() {
    use engine::{AlphaOutput as A, Codec as C, Container as K, ProResProfile as P};
    let mut codecs = vec![
        C::H264 {
            bitrate_bps: 16_000_000,
        },
        C::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit: false,
        },
        C::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit: true,
        },
        C::Vp9 {
            bitrate_bps: 16_000_000,
        },
        C::Vp8 {
            bitrate_bps: 16_000_000,
        },
        C::Png { sixteen_bit: false },
        C::Png { sixteen_bit: true },
    ];
    codecs.extend([P::Proxy, P::Lt, P::Standard, P::Hq, P::P4444, P::P4444Xq].map(C::ProRes));
    let mut accepted = BTreeSet::new();
    for codec in codecs {
        for container in [K::Mov, K::Mp4, K::Webm, K::PngSequence, K::PngStill] {
            for alpha in [A::None, A::Straight, A::Premultiplied] {
                for audio in [
                    None,
                    Some(AudioCodec::Pcm16),
                    Some(AudioCodec::Pcm24),
                    Some(AudioCodec::PcmF32),
                    Some(AudioCodec::Aac),
                    Some(AudioCodec::Opus),
                ] {
                    for timecode in [false, true] {
                        let mut s = spec(codec, container);
                        s.alpha = alpha;
                        s.audio = audio.map(|codec| AudioSpec {
                            sample_rate: 48_000,
                            channels: 2,
                            codec,
                        });
                        if timecode {
                            s.timecode = Some(TimecodeSpec {
                                start_frame: 0,
                                format: pm_time::TimecodeFormat::non_drop(s.rate),
                                reel: None,
                            });
                        }
                        if s.validate().is_ok() {
                            accepted.insert(tuple(&s));
                        }
                    }
                }
            }
        }
    }
    let mut reachable = BTreeSet::new();
    for format in Format::ALL.into_iter().filter(|f| *f != Format::Json) {
        for profile in [
            Profile::Proxy,
            Profile::Lt,
            Profile::Standard,
            Profile::Hq,
            Profile::P4444,
            Profile::P4444Xq,
        ] {
            let codecs: &[Option<VideoCodec>] = match format {
                Format::Prores => &[
                    Some(VideoCodec::Prores),
                    Some(VideoCodec::H264),
                    Some(VideoCodec::Hevc),
                    Some(VideoCodec::Hevc10),
                ],
                Format::Mp4 => &[
                    Some(VideoCodec::H264),
                    Some(VideoCodec::Hevc),
                    Some(VideoCodec::Hevc10),
                ],
                Format::Webm => &[Some(VideoCodec::Vp9), Some(VideoCodec::Vp8)],
                _ => &[None],
            };
            for &codec in codecs {
                for depth in [8, 16] {
                    let mut o = Options::new(640, 360, 30.);
                    o.format = format;
                    o.profile = profile;
                    o.video_codec = codec;
                    o.depth = depth;
                    for alpha in o.alpha_modes() {
                        o.alpha_mode = Some(alpha);
                        let codings: &[sheet::AudioCoding] = match format {
                            Format::Prores => &[
                                sheet::AudioCoding::Pcm16,
                                sheet::AudioCoding::Pcm24,
                                sheet::AudioCoding::PcmFloat,
                                sheet::AudioCoding::Aac,
                            ],
                            Format::Mp4 => &[sheet::AudioCoding::Aac],
                            _ => &[sheet::AudioCoding::Opus],
                        };
                        for &coding in codings {
                            o.audio_coding = coding;
                            for audio in [false, true] {
                                o.audio = audio;
                                for track in [false, true] {
                                    o.timecode_track = track;
                                    let s = from_sheet(&o).unwrap();
                                    s.validate().unwrap_or_else(|e| panic!("{o:?}: {e}"));
                                    reachable.insert(tuple(&s));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(accepted, reachable);
}

#[test]
fn invalid_explicit_selections_refuse_and_disabled_timecode_is_dormant() {
    let mut o = Options::new(640, 360, 30.);
    o.video_codec = Some(VideoCodec::Vp8);
    assert!(o.intent(true, 0).is_err());
    o.video_codec = Some(VideoCodec::H264);
    o.alpha_mode = Some(AlphaMode::Straight);
    assert!(from_sheet(&o).is_err());
    o.format = Format::Prores;
    o.alpha_mode = Some(AlphaMode::None);
    o.timecode = "invalid".into();
    o.timecode_track = false;
    assert!(from_sheet(&o).unwrap().timecode.is_none());
    o.timecode_track = true;
    assert!(from_sheet(&o).is_err());
    assert!(serde_json::from_value::<Options>(json!({"videoCodec":"unknown"})).is_err());
}

#[test]
fn ui_availability_table_matches_engine_rules() {
    let table: serde_json::Value =
        serde_json::from_str(include_str!("export_availability.json")).unwrap();
    for row in table.as_array().unwrap() {
        let mut value = serde_json::to_value(Options::new(640, 360, 30.)).unwrap();
        for (key, field) in row["options"].as_object().unwrap() {
            value[key] = field.clone();
        }
        let mut o: Options = serde_json::from_value(value).unwrap();
        o.audio = false;
        assert_eq!(
            serde_json::to_value(o.alpha_modes()).unwrap(),
            row["alphaModes"]
        );
        let mut audio = Vec::new();
        if o.format.video() {
            let base = from_sheet(&o).unwrap();
            for (name, codec) in [
                ("pcm24", AudioCodec::Pcm24),
                ("pcm16", AudioCodec::Pcm16),
                ("pcmFloat", AudioCodec::PcmF32),
                ("aac", AudioCodec::Aac),
                ("opus", AudioCodec::Opus),
            ] {
                let mut candidate = base.clone();
                candidate.audio = Some(AudioSpec {
                    sample_rate: 48_000,
                    channels: 2,
                    codec,
                });
                if candidate.validate().is_ok() {
                    audio.push(name);
                }
            }
        }
        assert_eq!(json!(audio), row["audioCodings"]);
        assert_eq!(o.reads("audioCoding"), audio.len() > 1);
        assert_eq!(o.reads("alpha"), o.alpha_modes().len() > 1);
        assert_eq!(o.reads("alphaMode"), o.alpha_modes().len() > 1);
        assert_eq!(
            o.reads("quality"),
            o.format.video() && o.engine_codec().bitrate_bps().is_some()
        );
        assert_eq!(json!(o.format == Format::Prores), row["timecode"]);
    }
}

#[test]
fn default_format_visibility_matches_effective_codec() {
    for format in Format::ALL {
        let mut options = Options::new(640, 360, 30.);
        options.format = format;
        for field in ["quality", "alpha", "audioCoding"] {
            assert_eq!(
                format.reads(field),
                options.reads(field),
                "{format:?}/{field}"
            );
        }
    }
    let mut options = Options::new(640, 360, 30.);
    options.format = Format::Prores;
    assert!(!options.reads("quality"));
    options.video_codec = Some(VideoCodec::H264);
    assert!(options.reads("quality"));
    assert!(!options.reads("alpha"));
    options.video_codec = Some(VideoCodec::Hevc);
    assert!(options.reads("alpha"));
    options.video_codec = Some(VideoCodec::Prores);
    options.profile = Profile::Hq;
    assert!(!options.reads("alpha"));
}
