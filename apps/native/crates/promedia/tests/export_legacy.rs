use pm_promedia::export_panel::Options;
use pm_protocol::{color::OutputColor, common::*, priority::*};
use serde_json::json;

fn base() -> ExportSettings {
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
        audio: true,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::Rec709Broadcast,
        precision: pm_protocol::color::Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

#[test]
fn legacy_mapping_table() {
    let mut rows = Vec::new();
    for format in ["prores", "mp4", "webm", "png", "still", "json"] {
        for profile in ["422proxy", "422lt", "422", "422hq", "4444", "4444xq"] {
            for codec in ["h264", "hevc"] {
                for alpha in [false, true] {
                    for audio in [false, true] {
                        for coding in ["pcm16", "pcm24", "pcmFloat"] {
                            for dynamic in ["sdr", "pq", "hlg"] {
                                let value = json!({"format":format,"profile":profile,"codec":codec,
                                    "alpha":alpha,"audio":audio,"audioCoding":coding,"dynamicRange":dynamic,
                                    "scale":1,"width":640,"height":360,"fps":30,"range":"all",
                                    "quality":"high","mblur":true,"captionsBurn":false,"captionsSidecar":"none",
                                    "timecode":"01:00:00:00"});
                                let options: Options =
                                    serde_json::from_value(value.clone()).unwrap();
                                let mut planned = base();
                                if format == "png" || format == "still" {
                                    planned.color = OutputColor::SrgbImage;
                                }
                                let settings = options
                                    .intent(true, 0)
                                    .unwrap()
                                    .engine_settings(planned)
                                    .unwrap();
                                let digest = blake3::hash(&serde_json::to_vec(&settings).unwrap())
                                    .to_hex()
                                    .to_string();
                                rows.push(json!([
                                    format, profile, codec, alpha, audio, coding, dynamic, digest
                                ]));
                            }
                        }
                    }
                }
            }
        }
    }
    let captured = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/export_legacy.json"
    ))
    .unwrap();
    let captured: Vec<serde_json::Value> = serde_json::from_str(&captured).unwrap();
    assert_eq!(rows, captured);
}
