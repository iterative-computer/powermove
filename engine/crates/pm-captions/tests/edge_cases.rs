use pm_captions::*;
use serde_json::{Value, json};
use std::collections::HashSet;
fn read(v: Value) -> Vec<CaptionCue> {
    normalize_cues(&v, &mut || "new".into())
}
#[test]
fn rebasing_and_clipping_preserve_composition_word_times_at_a_split() {
    let original = read(json!([
 {"id":"a","start":1,"end":2,"text":"One"},
 {"id":"b","start":3,"end":5,"text":"Two spans the cut","words":[{"text":"Two","start":3,"end":4.5},{"text":"cut","start":4.5,"end":5}]},
 {"id":"c","start":6,"end":7,"text":"Three"}]));
    let head = clip_cues_to_end(&original, 4.0);
    let tail = rebase_cues_for_start(&original, 4.0);
    assert_eq!(head.len(), 2);
    assert_eq!(tail.len(), 2);
    assert_eq!((head[1].start, head[1].end), (3.0, 4.0));
    assert_eq!((tail[0].start + 4.0, tail[0].end + 4.0), (4.0, 5.0));
    assert_eq!(head[1].words.as_ref().unwrap()[0].end, 4.0);
    assert_eq!(tail[0].words.as_ref().unwrap()[0].start, 0.0);
    assert_eq!(tail[0].words.as_ref().unwrap()[1].start + 4.0, 4.5);
    assert_eq!(original[1].end, 5.0);
    assert_eq!(rebase_cues_for_start(&original, -1.0)[0].start, 2.0);
}
#[test]
fn malformed_and_colliding_id_allocators_cannot_erase_or_duplicate_cues() {
    let raw = json!([{"start":0,"end":1,"text":"a"},{"id":"new","start":2,"end":3,"text":"b"},{"id":"new","start":4,"end":5,"text":"c"}]);
    for generated in ["new", ""] {
        let out = normalize_cues(&raw, &mut || generated.into());
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].id, "new");
        assert_eq!(out.iter().map(|c| &c.id).collect::<HashSet<_>>().len(), 3);
        assert!(out.iter().all(|c| !c.id.is_empty()));
    }
    let out = read(raw);
    assert_eq!(split_cue(&out, &out[0].id, 0.5, "new").1, None);
    let inserted = insert_cues(&out, &json!([{"start":6,"end":7,"text":"d"}]), &mut || {
        "new".into()
    });
    assert_eq!(
        inserted.iter().map(|c| &c.id).collect::<HashSet<_>>().len(),
        4
    );
}
#[test]
fn unicode_and_markup_text_round_trip_without_becoming_tags_or_entities() {
    let original = read(
        json!([{"id":"a","start":0.125,"end":2.75,"text":"<i>literal</i> &lt; & 你好 😀\nsecond line"}]),
    );
    for format in [CaptionFormat::Srt, CaptionFormat::Vtt] {
        let serialized = format_captions(format, &original, &WriteOptions::default());
        let back = parse_captions(&serialized, Some(format), "");
        assert_eq!(back.cues[0].text, original[0].text);
        assert_eq!((back.cues[0].start, back.cues[0].end), (0.125, 2.75));
    }
}
#[test]
fn whole_cue_moves_carry_words_and_text_changes_clear_stale_timings() {
    let source = read(
        json!([{"id":"a","start":1,"end":3,"text":"one two","words":[{"text":"one","start":1,"end":2},{"text":"two","start":2,"end":3}]}]),
    );
    let moved = update_cues(&source, &json!([{"id":"a","start":3,"end":5}]));
    assert_eq!(moved[0].words.as_ref().unwrap()[0].start, 3.0);
    assert!(
        update_cues(&moved, &json!([{"id":"a","text":"edited"}]))[0]
            .words
            .is_none()
    );
    assert!(
        update_cues(&source, &json!([{"id":"a","words":null}]))[0]
            .words
            .is_none()
    );
    assert!(update_cues(&source, &json!([{"id":"a","text":"  "}])).is_empty());
}
#[test]
fn exact_project_serialization_preserves_word_times_style_and_unknown_content() {
    let original = normalize_captions_content(
        &json!({"cues":[{"id":"a","start":0.123456,"end":3.234567,"text":"hello","words":[{"text":"hello","start":0.234567,"end":2.987654}]}],"language":"zh-Hans-CN","customExtensionData":{"revision":7},"style":{"preset":"pop","size":64}}),
        &mut || "id".into(),
    );
    let back: CaptionsContent =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    assert_eq!(back, original);
    assert_eq!(back.extra["customExtensionData"]["revision"], 7);
    assert!(valid_language("en-US"));
    assert!(!valid_language("en\nLanguage:xx"));
}
#[test]
fn normalization_is_stable_for_many_overlaps_and_never_mutates_input() {
    let mut state = 4u64;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state >> 32) as usize
    };
    for _ in 0..200 {
        let raw:Value=(0..40).map(|i|{let start=(next()%1000) as f64/100.0;json!({"id":format!("c{i}"),"start":start,"end":start+(next()%400+4) as f64/100.0,"text":format!("word{i}")})}).collect();
        let before = raw.clone();
        let normalized = read(raw.clone());
        assert_eq!(raw, before);
        let again = read(serde_json::to_value(&normalized).unwrap());
        assert_eq!(normalized, again);
        assert!(normalized.windows(2).all(|w| w[0].end <= w[1].start + 1e-6));
        assert!(
            normalized
                .iter()
                .all(|c| c.end - c.start >= MIN_CUE_DURATION - 1e-6)
        );
    }
}
#[test]
fn header_inline_timing_ruby_entities_and_cr_line_endings_parse() {
    let parsed = parse_vtt(
        "\u{feff}WEBVTT\r\rNOTE x\rignored\r\r00:01.000 --> 00:03.000 align:start\r<ruby>漢<rt>kan</rt></ruby> <00:02.000>&#x1f600;",
    );
    assert_eq!(parsed.skipped, 0);
    assert_eq!(parsed.cues[0].text, "漢 😀");
    let words = parsed.cues[0].words.as_ref().unwrap();
    assert_eq!(words[0].end, 2.0);
    assert_eq!(words[1].text, "😀");
}
#[test]
fn export_never_writes_a_zero_or_reversed_interval_after_negative_rebase() {
    let cues = read(json!([{"id":"a","start":0,"end":1,"text":"a"}]));
    let out = format_srt(
        &cues,
        &WriteOptions {
            rebase: 2.0,
            ..Default::default()
        },
    );
    assert!(out.is_empty());
    let partial = format_srt(
        &cues,
        &WriteOptions {
            rebase: 0.5,
            ..Default::default()
        },
    );
    let back = parse_srt(&partial);
    assert_eq!((back.cues[0].start, back.cues[0].end), (0.0, 0.5));
}
#[test]
fn non_finite_edit_times_do_not_poison_cue_lists() {
    let cues = read(json!([{"id":"a","start":1,"end":2,"text":"a"}]));
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(rebase_cues_for_start(&cues, n), cues);
        assert_eq!(clip_cues_to_end(&cues, n), cues);
        assert_eq!(move_cues(&cues, &["a".into()], n), cues);
        assert_eq!(split_cue(&cues, "a", n, "tail").1, None);
        assert_eq!(cue_index_at(&cues, n), None);
    }
}

#[test]
fn long_hour_fields_are_supported_and_unparseable_numbers_cannot_panic() {
    assert_eq!(parse_timestamp("1000:00:01.000"), Some(3_600_001.0));
    assert_eq!(parse_timestamp("１２:３４.５"), None);
    assert_eq!(parse_timestamp(&("9".repeat(400) + ":00:01.000")), None);
    let cues =
        read(json!([{"id":"a","start":3_600_000.125,"end":3_600_001.75,"text":"long timeline"}]));
    for fmt in [CaptionFormat::Srt, CaptionFormat::Vtt] {
        let back = parse_captions(
            &format_captions(fmt, &cues, &WriteOptions::default()),
            Some(fmt),
            "",
        );
        assert_eq!(back.cues[0].start, cues[0].start);
    }
}

#[test]
fn json_number_and_text_coercion_match_the_extension_contract() {
    let cues = read(
        json!([{"start":"0x10","end":"0b10011","text":42.0},{"start":[20],"end":[21],"text":"\u{feff} caption \u{feff}"}]),
    );
    assert_eq!((cues[0].start, cues[0].end), (16.0, 19.0));
    assert_eq!(cues[0].text, "42");
    assert_eq!(cues[1].text, "caption");
    assert_eq!(clean_cue_text(&json!(["a", null, "b"])), "a,,b");
}
