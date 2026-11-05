use pm_model::json::parse;
use pm_model::schema::{Codec, Project};

#[test]
fn main_data_fields_are_typed_and_lossless_including_nested_notes_and_caption_words() {
    let value = parse(include_str!("fixtures/main-delta/project.json")).unwrap();
    let project = Project::decode(value.clone()).unwrap();
    assert!(project.encode().identical(&value));
    assert_eq!(project.color_depth, Some(32.0));
    assert_eq!(project.memory_enabled, Some(false));
    assert_eq!(project.memory.as_ref().unwrap().len(), 2);
    assert_eq!(project.audio_gain, Some(0.625));
    let nested = &project.comps.as_ref().unwrap()[b"C2".as_slice()];
    assert_eq!(nested.memory_enabled, Some(false));
    assert_eq!(nested.memory.as_ref().unwrap().len(), 1);
    let captions = project.layers.as_ref().unwrap()[1].d.as_ref().unwrap();
    let cue = &captions.cues.as_ref().unwrap()[0];
    assert_eq!(cue.start, Some(1.25));
    assert_eq!(cue.words.as_ref().unwrap()[0].end, Some(1.8));
    assert_eq!(project.export_defaults.unwrap().format.unwrap(), "web");
}

#[test]
fn malformed_or_future_memory_and_precision_are_never_normalized_away() {
    for text in [
        r#"{"memory":null,"memoryEnabled":"future","colorDepth":"hdr"}"#,
        r#"{"memory":[{"id":9,"text":null,"kind":"future","createdAt":{}}],"colorDepth":64}"#,
        r#"{"memory":[{"id":"x","text":"\ud800","author":"future"}],"memoryEnabled":false}"#,
    ] {
        let value = parse(text).unwrap();
        let typed = Project::decode(value.clone()).unwrap();
        assert!(typed.encode().identical(&value));
    }
}
