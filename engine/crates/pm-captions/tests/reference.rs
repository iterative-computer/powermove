//! Frozen, public v1.2.0 fixtures. Node is only needed to regenerate these;
//! cargo tests and the product runtime are entirely Rust.
use pm_captions::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
fn fixtures() -> Value {
    fn expand(v: &Value, shared: &Value) -> Value {
        if let Some(key) = v.get("$fixture").and_then(Value::as_str) {
            return shared[key].clone();
        }
        match v {
            Value::Array(a) => a.iter().map(|v| expand(v, shared)).collect(),
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, v)| (k.clone(), expand(v, shared)))
                    .collect(),
            ),
            v => v.clone(),
        }
    }
    let mut v: Value = serde_json::from_str(include_str!("reference.json")).unwrap();
    v["cases"] = expand(&v["cases"], &v["shared"]);
    v
}
fn cues(v: &Value) -> Vec<CaptionCue> {
    serde_json::from_value(v.clone()).unwrap()
}
fn strings(v: &Value) -> Vec<String> {
    serde_json::from_value(v.clone()).unwrap()
}
fn no_ids(v: Value) -> Value {
    match v {
        Value::Array(a) => a.into_iter().map(no_ids).collect(),
        Value::Object(mut m) => {
            if m.contains_key("id")
                && m.contains_key("text")
                && m.contains_key("start")
                && m.contains_key("end")
            {
                m.remove("id");
            }
            Value::Object(m.into_iter().map(|(k, v)| (k, no_ids(v))).collect())
        }
        v => v,
    }
}
fn assert_json(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => assert!(
            (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-9,
            "{path}: {a} != {b}"
        ),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: array length");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                assert_json(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{path}: object keys"
            );
            for (k, a) in a {
                assert_json(a, &b[k], &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}
fn format(v: &Value) -> CaptionFormat {
    match v.as_str().unwrap() {
        "vtt" => CaptionFormat::Vtt,
        _ => CaptionFormat::Srt,
    }
}
fn run(case: &Value) -> Value {
    let a = case["args"].as_array().unwrap();
    let n = |i: usize| a[i].as_f64().unwrap();
    let s = |i: usize| a[i].as_str().unwrap();
    let mut id = 0;
    let mut make = || {
        id += 1;
        format!("n{id}")
    };
    match case["op"].as_str().unwrap() {
        "normalizeCues" => json!(normalize_cues(&a[0], &mut make)),
        "normalizeCaptionsContent" => json!(normalize_captions_content(
            a.first().unwrap_or(&Value::Null),
            &mut make
        )),
        "normalizeCaptionStyle" => json!(normalize_caption_style(&a[0])),
        "presetStyle" => json!(preset_style(
            s(0),
            a.get(1).and_then(Value::as_f64).unwrap_or(1080.0)
        )),
        "patchStyle" => json!(patch_style(
            &serde_json::from_value(a[0].clone()).unwrap(),
            &a[1],
            a.get(2).and_then(Value::as_f64).unwrap_or(1080.0)
        )),
        "cueIndexAt" => json!(
            cue_index_at(&cues(&a[0]), n(1))
                .map(|i| i as i64)
                .unwrap_or(-1)
        ),
        "cueRange" => json!(cue_range(&cues(&a[0]), n(1), n(2))),
        "moveLimits" => json!(move_limits(&cues(&a[0]), &strings(&a[1]))),
        "moveCues" => json!(move_cues(&cues(&a[0]), &strings(&a[1]), n(2))),
        "trimCue" => json!(trim_cue(
            &cues(&a[0]),
            s(1),
            if s(2) == "start" {
                CueEdge::Start
            } else {
                CueEdge::End
            },
            n(3)
        )),
        "updateCues" => json!(update_cues(&cues(&a[0]), &a[1])),
        "insertCues" => json!(insert_cues(&cues(&a[0]), &a[1], &mut make)),
        "deleteCues" => json!(delete_cues(&cues(&a[0]), &strings(&a[1]))),
        "splitCue" => {
            let (c, tail) = split_cue(
                &cues(&a[0]),
                s(1),
                n(2),
                case["withoutIds"]["tailId"].as_str().unwrap_or("new"),
            );
            json!({"cues":c,"tailId":tail})
        }
        "mergeCues" => {
            let (c, id) = merge_cues(&cues(&a[0]), &strings(&a[1]));
            json!({"cues":c,"id":id})
        }
        "joinWords" => json!(join_words(
            a[0].as_array().unwrap().iter().map(|s| s.as_str().unwrap())
        )),
        "rebaseCuesForStart" => json!(rebase_cues_for_start(&cues(&a[0]), n(1))),
        "clipCuesToEnd" => json!(clip_cues_to_end(&cues(&a[0]), n(1))),
        "shiftAllCues" => json!(shift_all_cues(&cues(&a[0]), n(1))),
        "cueWords" => json!(cue_words(&serde_json::from_value(a[0].clone()).unwrap())),
        "parseSrt" => json!(parse_srt(s(0))),
        "parseVtt" => json!(parse_vtt(s(0))),
        "parseCaptions" => json!(parse_captions(
            s(0),
            a.get(1).filter(|v| v.as_str() != Some("auto")).map(format),
            a.get(2).and_then(Value::as_str).unwrap_or("")
        )),
        "formatSrt" => json!(format_srt(
            &cues(&a[0]),
            &serde_json::from_value(a.get(1).cloned().unwrap_or_else(|| json!({}))).unwrap()
        )),
        "formatVtt" => json!(format_vtt(
            &cues(&a[0]),
            &serde_json::from_value(a.get(1).cloned().unwrap_or_else(|| json!({}))).unwrap()
        )),
        "detectCaptionFormat" => json!(detect_caption_format(
            s(0),
            a.get(1).and_then(Value::as_str).unwrap_or("")
        )),
        "parseTimestamp" => json!(parse_timestamp(s(0))),
        "stripMarkup" => json!(strip_markup(s(0))),

        "displayLength" => json!(display_length(s(0))),
        "textTokens" => json!(text_tokens(s(0))),
        op => panic!("unhandled reference operation: {op}"),
    }
}
fn check(op: &str) {
    let f = fixtures();
    let cases: Vec<_> = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["op"] == op)
        .collect();
    assert!(!cases.is_empty(), "missing {op} oracle");
    for (i, c) in cases.into_iter().enumerate() {
        assert_json(&no_ids(run(c)), &c["withoutIds"], &format!("{op}[{i}]"));
    }
}
macro_rules! reference_tests {($($name:ident=>$op:literal),*$(,)?)=>{$(#[test]fn $name(){check($op);})*};}
reference_tests! {
 normalize_cue_model=>"normalizeCues",normalize_content=>"normalizeCaptionsContent",normalize_styles=>"normalizeCaptionStyle",presets=>"presetStyle",style_patches=>"patchStyle",lookup=>"cueIndexAt",ranges=>"cueRange",move_bounds=>"moveLimits",moves=>"moveCues",trims=>"trimCue",updates=>"updateCues",inserts=>"insertCues",deletes=>"deleteCues",splits=>"splitCue",merges=>"mergeCues",words=>"cueWords",srt_import=>"parseSrt",vtt_import=>"parseVtt",auto_import=>"parseCaptions",srt_export=>"formatSrt",vtt_export=>"formatVtt",detection=>"detectCaptionFormat",timestamps=>"parseTimestamp",markup=>"stripMarkup",cjk_join=>"joinWords",display_width=>"displayLength",text_tokenization=>"textTokens",clip_rebase=>"rebaseCuesForStart",clip_end=>"clipCuesToEnd",shift_all=>"shiftAllCues"
}
#[test]
fn every_subtitle_fixture_round_trips_both_formats_and_lossless_project_json() {
    let f = fixtures();
    let mut count = 0;
    for c in f["cases"].as_array().unwrap() {
        if !c["op"].as_str().unwrap().starts_with("parse") || c["op"] == "parseTimestamp" {
            continue;
        }
        let parsed: ParsedCaptions = serde_json::from_value(run(c)).unwrap();
        let encoded = serde_json::to_string(&parsed.cues).unwrap();
        assert_eq!(cues(&serde_json::from_str(&encoded).unwrap()), parsed.cues);
        for fmt in [CaptionFormat::Srt, CaptionFormat::Vtt] {
            let serialized = format_captions(fmt, &parsed.cues, &WriteOptions::default());
            let back = parse_captions(&serialized, Some(fmt), "");
            let spans = |cs: &[CaptionCue]| {
                cs.iter()
                    .map(|c| json!([c.start, c.end, c.text]))
                    .collect::<Vec<_>>()
            };
            assert_eq!(spans(&back.cues), spans(&parsed.cues), "{}", c["op"]);
            assert_eq!(back.skipped, 0);
            assert_eq!(
                back.cues
                    .iter()
                    .map(|c| &c.id)
                    .collect::<BTreeSet<_>>()
                    .len(),
                back.cues.len()
            );
        }
        count += 1;
    }
    assert!(count >= 15);
}
