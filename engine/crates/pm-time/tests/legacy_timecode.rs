//! pm-time timecode against the editor's own helpers
//! (`legacy/core/timecode.ts`: `formatDropFrameTimecode`,
//! `parseDropFrameTimecode`, `timecodeBase`), through
//! tests/vectors/timecode-vectors.json written by
//! tests/vectors/generate-timecode-vectors.ts.

use pm_time::{Rate, TimecodeFormat};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/timecode-vectors.json")).unwrap()
}

fn int(v: &Value) -> i64 {
    v.as_f64().map(|f| f as i64).unwrap()
}

#[test]
fn drop_frame_matches_the_editor() {
    let vectors = vectors();
    let mut checked = 0;
    for scheme in vectors["dropFrame"].as_array().unwrap() {
        let rate = Rate::new(int(&scheme["num"]), int(&scheme["den"])).unwrap();
        let format = TimecodeFormat::drop_frame(rate).unwrap();
        for case in scheme["format"].as_array().unwrap() {
            let frame = int(&case["frame"]);
            assert_eq!(
                format.format(frame),
                case["text"].as_str().unwrap(),
                "{rate} frame {frame}"
            );
            checked += 1;
        }
        for case in scheme["parse"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let expected = case["frame"].as_f64().map(|f| f as i64);
            assert_eq!(format.parse(text).ok(), expected, "{rate} {text:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, 12_000);
}

#[test]
fn label_bases_match_the_editor() {
    for case in vectors()["bases"].as_array().unwrap() {
        let rate = Rate::new(int(&case["num"]), int(&case["den"])).unwrap();
        assert_eq!(
            TimecodeFormat::non_drop(rate).base() as i64,
            int(&case["base"]),
            "{rate}"
        );
    }
}
