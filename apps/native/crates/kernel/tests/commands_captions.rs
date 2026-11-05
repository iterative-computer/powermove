use pm_kernel::commands::captions::{
    AddCaptions, CaptionCommand, CaptionEdit, CaptionFormat, CueSource,
};
use pm_kernel::{Command, ContentCommand, DocumentStore, Origin};
use pm_model::json::{Value, parse};

fn store() -> DocumentStore {
    DocumentStore::new_untitled(parse(r#"{"id":"P","name":"Synthetic","w":1920,"h":1080,"fps":25,"dur":20,"layers":[],"comps":{},"assets":{}}"#).unwrap())
}
fn add(id: &str) -> Command {
    CaptionCommand::Add(Box::new(AddCaptions {
        id:id.into(),name:None,source:CueSource::Cues(parse(r#"[{"id":"a","start":4,"end":5,"text":"First"},{"id":"b","start":6,"end":7,"text":"Second"}]"#).unwrap()),
        style:None,language:Some("en".into()),from:Some(3.),duration:Some(4.),index:None,comp:None,
    })).into()
}
fn layer(store: &DocumentStore) -> Value {
    store
        .root()
        .unwrap()
        .get("layers")
        .unwrap()
        .at(0)
        .unwrap()
        .to_value()
}
fn data(layer: &Value) -> &Value {
    layer.get("d").unwrap()
}
fn cues(layer: &Value) -> &[Value] {
    data(layer).get("cues").unwrap().as_array().unwrap()
}
fn edit(store: &mut DocumentStore, operation: CaptionEdit) {
    store
        .execute(
            CaptionCommand::Edit {
                layer_id: "caption".into(),
                edit: operation,
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
}
#[test]
fn add_uses_composition_time_and_real_history() {
    let mut store = store();
    store.execute(add("caption"), Origin::Interface).unwrap();
    let l = layer(&store);
    assert_eq!(l.get("type").and_then(Value::as_str), Some("captions"));
    for (key, value) in [
        ("opacity", 100.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("position.x", 960.),
        ("position.y", 540.),
    ] {
        assert_eq!(
            l.get("p")
                .unwrap()
                .get(key)
                .unwrap()
                .get("v")
                .and_then(Value::as_f64),
            Some(value),
            "{key}"
        );
    }
    assert_eq!(cues(&l)[0].get("start").and_then(Value::as_f64), Some(1.));
    assert_eq!(cues(&l)[1].get("end").and_then(Value::as_f64), Some(4.));
    store.undo().unwrap();
    assert!(store.layers().is_empty());
    store.redo().unwrap();
    assert_eq!(layer(&store), l);
}
#[test]
fn update_split_merge_move_delete_and_replace_are_real_edits() {
    let mut store = store();
    store.execute(add("caption"), Origin::Interface).unwrap();
    edit(
        &mut store,
        CaptionEdit::Update(parse(r#"[{"id":"a","text":"Changed","end":5.5}]"#).unwrap()),
    );
    assert_eq!(
        cues(&layer(&store))[0].get("end").and_then(Value::as_f64),
        Some(2.5)
    );
    edit(
        &mut store,
        CaptionEdit::Split {
            id: "a".into(),
            at: 4.5,
        },
    );
    assert_eq!(cues(&layer(&store)).len(), 3);
    let ids = cues(&layer(&store))[..2]
        .iter()
        .map(|c| c.get("id").unwrap().as_str().unwrap().to_owned())
        .collect();
    edit(&mut store, CaptionEdit::Merge(ids));
    assert_eq!(cues(&layer(&store)).len(), 2);
    edit(
        &mut store,
        CaptionEdit::Move {
            ids: vec!["b".into()],
            by: 1.,
        },
    );
    assert_eq!(layer(&store).get("dur").and_then(Value::as_f64), Some(5.));
    edit(&mut store, CaptionEdit::Delete(vec!["b".into()]));
    assert_eq!(cues(&layer(&store)).len(), 1);
    edit(
        &mut store,
        CaptionEdit::Replace(CueSource::Cues(
            parse(r#"[{"id":"c","start":8,"end":9,"text":"New"}]"#).unwrap(),
        )),
    );
    assert_eq!(
        cues(&layer(&store))[0].get("start").and_then(Value::as_f64),
        Some(5.)
    );
}
#[test]
fn text_and_style_edits_do_not_reveal_trimmed_tail_cues() {
    let mut store = store();
    store.execute(add("caption"), Origin::Interface).unwrap();
    store
        .execute(
            pm_kernel::LayerCommand::Set {
                layer_id: "caption".into(),
                patch: pm_kernel::commands::layers::LayerPatch {
                    duration: Some(2.),
                    ..Default::default()
                },
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
    edit(
        &mut store,
        CaptionEdit::Update(parse(r#"[{"id":"a","text":"Typed"}]"#).unwrap()),
    );
    edit(
        &mut store,
        CaptionEdit::Style {
            style: parse(r#"{"size":75}"#).unwrap(),
            language: Some("fr".into()),
        },
    );
    let l = layer(&store);
    assert_eq!(l.get("dur").and_then(Value::as_f64), Some(2.));
    assert_eq!(data(&l).get("language").and_then(Value::as_str), Some("fr"));
    assert_eq!(
        data(&l)
            .get("style")
            .unwrap()
            .get("size")
            .and_then(Value::as_f64),
        Some(75.)
    );
}
#[test]
fn invalid_and_locked_edits_leave_everything_unchanged() {
    let mut store = store();
    store.execute(add("caption"), Origin::Interface).unwrap();
    for operation in [
        CaptionEdit::Split {
            id: "a".into(),
            at: 4.,
        },
        CaptionEdit::Merge(vec!["a".into()]),
        CaptionEdit::Update(parse(r#"[{"id":"missing","text":"x"}]"#).unwrap()),
        CaptionEdit::Move {
            ids: vec!["a".into()],
            by: f64::NAN,
        },
    ] {
        let before = store.snapshot();
        assert!(
            store
                .execute(
                    CaptionCommand::Edit {
                        layer_id: "caption".into(),
                        edit: operation
                    }
                    .into(),
                    Origin::Agent
                )
                .is_err()
        );
        assert_eq!(store.revision(), before.revision);
        assert_eq!(
            store.root().unwrap().to_value(),
            before.document.to_value().unwrap()
        );
    }
    store
        .execute(
            Command::SetLayerLocked {
                layer_id: "caption".into(),
                locked: true,
            },
            Origin::Interface,
        )
        .unwrap();
    assert!(
        store
            .execute(
                CaptionCommand::Edit {
                    layer_id: "caption".into(),
                    edit: CaptionEdit::Delete(vec!["a".into()])
                }
                .into(),
                Origin::Interface
            )
            .is_err()
    );
}
#[test]
fn content_patch_normalizes_and_preserves_opaque_fields() {
    let mut project =
        pm_model::canonical::realize_project(store().snapshot().document.to_value().unwrap());
    let mut s = store();
    s.execute(add("caption"), Origin::Interface).unwrap();
    let mut l = layer(&s);
    l.as_object_mut()
        .unwrap()
        .get_mut("d")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("future", parse(r#"{"\ud800":"\udfff"}"#).unwrap());
    project
        .as_object_mut()
        .unwrap()
        .insert("layers", Value::Array(vec![l]));
    let mut s = DocumentStore::new_untitled(project);
    let patch=parse(r#"{"style":{"size":81},"cues":[{"id":"b","start":3,"end":4,"text":"B"},{"id":"a","start":1,"end":2,"text":"A"}]}"#).unwrap();
    s.execute(
        ContentCommand::Set {
            layer_id: "caption".into(),
            patch: patch.as_object().unwrap().clone(),
            time: 0.,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
    let l = layer(&s);
    assert_eq!(cues(&l)[0].get("id").and_then(Value::as_str), Some("a"));
    assert_eq!(
        data(&l).get("future"),
        parse(r#"{"\ud800":"\udfff"}"#).ok().as_ref()
    );
}
#[test]
fn srt_import_offsets_and_overlap_placement_are_admitted() {
    let mut s = store();
    s.execute(add("caption"), Origin::Interface).unwrap();
    s.execute(add("second"), Origin::Interface).unwrap();
    let l = layer(&s);
    assert_eq!(
        data(&l)
            .get("style")
            .unwrap()
            .get("placement")
            .and_then(Value::as_str),
        Some("top")
    );
    edit(
        &mut s,
        CaptionEdit::Import {
            source: CueSource::Text {
                text: "1\n00:00:04,000 --> 00:00:05,000\nImported\n".into(),
                format: CaptionFormat::Srt,
                offset: 1.,
            },
            replace: true,
        },
    );
    let root = s.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "caption").unwrap();
    let l = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
    assert_eq!(cues(&l)[0].get("start").and_then(Value::as_f64), Some(2.));
}

#[test]
fn generic_caption_layer_uses_visible_base_transform_and_height_scaled_content() {
    let mut store = store();
    let mut command = pm_kernel::commands::layers::AddLayer {
        id: "caption".into(),
        layer_type: "captions".into(),
        comp: None,
        name: None,
        from: None,
        duration: None,
        content: Default::default(),
        properties: Default::default(),
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time: 0.,
    };
    command.content=parse(r#"{"cues":[{"id":"a","start":1,"end":2,"text":"Synthetic"}],"style":{"size":72},"future":{"keep":true}}"#).unwrap().as_object().unwrap().clone();
    store
        .execute(
            pm_kernel::LayerCommand::Add(Box::new(command)).into(),
            Origin::Interface,
        )
        .unwrap();
    let l = layer(&store);
    assert_eq!(
        l.get("p").unwrap().get("opacity").unwrap().get("v"),
        Some(&Value::Number(100.))
    );
    assert_eq!(cues(&l).len(), 1);
    assert_eq!(
        data(&l).get("style").unwrap().get("size"),
        Some(&Value::Number(72.))
    );
    assert!(data(&l).get("future").is_some());
}
