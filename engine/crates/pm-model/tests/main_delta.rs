use pm_model::json::parse;
use pm_model::schema::{Codec, Project};
use pm_model::{
    Value,
    memory::{self, Context, Operation},
};

fn context(id: &str, now: f64) -> Context {
    Context {
        id: id.into(),
        now,
        author: "agent",
    }
}
fn add(kind: &str, text: &str) -> Operation {
    Operation::Add {
        kind: kind.into(),
        text: text.into(),
        why: None,
        resolved: false,
    }
}

#[test]
fn memory_singleton_index_and_disabled_data_follow_main_without_edit_revision_changes() {
    let original = parse(r#"{"revision":9,"edits":[{"id":"edit"}],"unknown":true}"#).unwrap();
    let (project, _) = memory::apply(
        &original,
        &add("goal", "  Exact intent  "),
        &context("goal", 1.0),
    )
    .unwrap();
    let (project, entry) = memory::apply(
        &project,
        &add("goal", "Updated intent"),
        &context("unused", 2.0),
    )
    .unwrap();
    assert_eq!(entry.unwrap().get("id").unwrap().as_str(), Some("goal"));
    let (project, _) =
        memory::apply(&project, &add("open", "Unfinished"), &context("open", 3.0)).unwrap();
    let (project, _) = memory::apply(
        &project,
        &add("decision", "Recent"),
        &context("decision", 4.0),
    )
    .unwrap();
    let index = memory::index(&project, 8);
    assert_eq!(index.total, 3);
    assert_eq!(index.open.len(), 1);
    assert_eq!(index.recent.len(), 1);
    assert_eq!(project.get("revision"), original.get("revision"));
    assert_eq!(project.get("edits"), original.get("edits"));
    let mut disabled = project.clone();
    disabled
        .as_object_mut()
        .unwrap()
        .insert("memoryEnabled", Value::Bool(false));
    assert!(!memory::enabled(&disabled));
    assert!(memory::apply(&disabled, &add("open", "No"), &context("no", 5.0)).is_err());
    assert_eq!(memory::list(&disabled, None, 100).len(), 3);
}

#[test]
fn memory_removal_and_cap_archive_unknown_and_evicted_notes() {
    let mut project = parse(r#"{"memory":[{"id":"future","kind":"future","unknown":true}],"nativeMemoryArchive":{"older":true}}"#).unwrap();
    for n in 0..102 {
        (project, _) = memory::apply(
            &project,
            &add(if n == 0 { "goal" } else { "decision" }, "Authored"),
            &context(&format!("n{n}"), n as f64),
        )
        .unwrap();
    }
    assert_eq!(memory::index(&project, 8).total, 100);
    assert!(
        project
            .get("memory")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.get("unknown") == Some(&Value::Bool(true)))
    );
    assert_eq!(
        project
            .get("nativeMemoryArchive")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let (removed, _) = memory::apply(
        &project,
        &Operation::Remove { id: "n0".into() },
        &context("unused", 103.0),
    )
    .unwrap();
    assert!(memory::index(&removed, 8).goal.is_none());
    assert_eq!(
        removed
            .get("nativeMemoryArchive")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(
        memory::apply(
            &removed,
            &Operation::Remove {
                id: "missing".into()
            },
            &context("unused", 104.0)
        )
        .is_err()
    );
}

#[test]
fn memory_update_keeps_unknowns_and_matches_utf16_limits_and_goal_conflicts() {
    let project = parse(r#"{"memory":[{"id":"g","kind":"goal","text":"Goal"},{"id":"d","kind":"decision","text":"Decision","future":true}]}"#).unwrap();
    let update = Operation::Update {
        id: "d".into(),
        kind: Some("goal".into()),
        text: None,
        why: None,
        resolved: None,
    };
    assert!(memory::apply(&project, &update, &context("unused", 1.0)).is_err());
    let update = Operation::Update {
        id: "d".into(),
        kind: None,
        text: Some(format!(" {} ", "😀".repeat(400)).into()),
        why: Some("  reason  ".into()),
        resolved: Some(true),
    };
    let (updated, entry) = memory::apply(&project, &update, &context("unused", 2.0)).unwrap();
    let entry = entry.unwrap();
    assert_eq!(
        entry
            .get("text")
            .unwrap()
            .as_js_string()
            .unwrap()
            .utf16_len(),
        600
    );
    assert_eq!(entry.get("future"), Some(&Value::Bool(true)));
    assert_eq!(memory::list(&updated, Some("decision"), 1)[0], entry);
}

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
