mod common;
use common::*;
use pm_fx_defs::audio::{Validated, parse};
use pm_kernel::{AudioEffectCommand as Audio, Command, DocumentStore, Origin};
use pm_model::json::parse as model_parse;
use serde_json::json;
fn store() -> DocumentStore {
    DocumentStore::new_untitled(model_parse(r##"{"id":"P","name":"Audio","compId":"C","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"sound","type":"audio","name":"Sound","from":0,"dur":1,"on":true,"d":{"asset":"s","gain":1}}]}"##).unwrap())
}
fn definition() -> Validated {
    parse(json!({"id":"test.gain","label":"My Gain","group":"Audio","params":[{"type":"number","k":"gain","label":"Gain","min":-96,"max":24,"unit":"dB","def":0}],"graph":[{"type":"gain","gainDb":{"param":"gain"}}]})).unwrap()
}
fn root(s: &DocumentStore) -> String {
    pm_model::json::to_canonical_string(&s.root().unwrap().to_value())
}
fn run(s: &mut DocumentStore, c: Audio) {
    s.execute(Command::AudioEffect(c), Origin::Interface)
        .unwrap();
}
fn roundtrip(s: &mut DocumentStore, c: Audio) {
    let before = root(s);
    run(s, c);
    let after = root(s);
    assert_ne!(before, after);
    s.undo().unwrap();
    assert_eq!(root(s), before);
    s.redo().unwrap();
    assert_eq!(root(s), after);
}
#[test]
fn every_stack_edit_undoes_and_redoes_exactly() {
    let mut s = store();
    roundtrip(
        &mut s,
        Audio::Add {
            layer_id: "sound".into(),
            effect_id: "f1".into(),
            definition: definition(),
        },
    );
    roundtrip(
        &mut s,
        Audio::Add {
            layer_id: "sound".into(),
            effect_id: "f2".into(),
            definition: definition(),
        },
    );
    roundtrip(
        &mut s,
        Audio::Reorder {
            layer_id: "sound".into(),
            effect_id: "f1".into(),
            index: 1,
        },
    );
    roundtrip(
        &mut s,
        Audio::Enable {
            layer_id: "sound".into(),
            effect_id: "f1".into(),
            enabled: false,
        },
    );
    roundtrip(
        &mut s,
        Audio::SetParameter {
            layer_id: "sound".into(),
            effect_id: "f1".into(),
            definition: definition(),
            key: "gain".into(),
            value: json!(-12),
        },
    );
    roundtrip(
        &mut s,
        Audio::Remove {
            layer_id: "sound".into(),
            effect_id: "f2".into(),
        },
    );
}
#[test]
fn audio_stack_survives_real_project_save_and_reopen() {
    let dir = TempDir::new("audio-effects");
    let mut s = store();
    run(
        &mut s,
        Audio::Add {
            layer_id: "sound".into(),
            effect_id: "f1".into(),
            definition: definition(),
        },
    );
    run(
        &mut s,
        Audio::SetParameter {
            layer_id: "sound".into(),
            effect_id: "f1".into(),
            definition: definition(),
            key: "gain".into(),
            value: json!(-6),
        },
    );
    let path = dir.path("audio.pmv");
    save_as(&mut s, &path).unwrap();
    let reopened = open_store(&path);
    let loc = pm_kernel::model::find_layer(reopened.root().unwrap(), "sound").unwrap();
    let layer = pm_kernel::model::layer_at(reopened.root().unwrap(), &loc)
        .unwrap()
        .to_value();
    let saved =
        serde_json::from_str::<serde_json::Value>(&pm_model::json::to_canonical_string(&layer))
            .unwrap();
    assert_eq!(
        saved["audioFx"],
        json!([{"id":"f1","type":"test.gain","name":"My Gain","on":true,"p":{"gain":-6}}])
    );
}
#[test]
fn invalid_parameter_edit_is_atomic_and_named() {
    let mut s = store();
    run(
        &mut s,
        Audio::Add {
            layer_id: "sound".into(),
            effect_id: "f".into(),
            definition: definition(),
        },
    );
    let before = root(&s);
    let error = s
        .execute(
            Command::AudioEffect(Audio::SetParameter {
                layer_id: "sound".into(),
                effect_id: "f".into(),
                definition: definition(),
                key: "gain".into(),
                value: json!(25),
            }),
            Origin::Interface,
        )
        .unwrap_err();
    assert!(error.to_string().contains("gain"));
    assert_eq!(root(&s), before);
}

#[test]
fn kernel_registry_rejects_invalid_audio_definitions_by_field() {
    use pm_kernel::state::{Registration, StateOp, StateSnapshot, StateTransaction};
    let mut value = serde_json::to_value(definition().definition()).unwrap();
    value["graph"][0]["gainDb"] = json!({"value":1000});
    let error = StateSnapshot::default()
        .stage(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::Register {
                registry: "audioEffects".into(),
                entry: Registration {
                    token: "invalid-audio".into(),
                    owner: pm_kernel::Writer {
                        realm: 1,
                        generation: 1,
                    },
                    id: "test.gain".into(),
                    definition: model_parse(&value.to_string()).unwrap(),
                },
            }],
        })
        .unwrap_err();
    assert!(error.contains("graph[0].gainDb"), "{error}");
}
