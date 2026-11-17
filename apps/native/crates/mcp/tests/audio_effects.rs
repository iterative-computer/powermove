use pm_kernel::state::{Registration, StateOp, StateSnapshot, StateTransaction};
use pm_kernel::{DocumentStore, Origin};
use serde_json::{Value, json};
fn model(v: &Value) -> pm_model::Value {
    pm_model::json::parse(&v.to_string()).unwrap()
}
fn document(s: &DocumentStore) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(
        &s.root().unwrap().to_value(),
    ))
    .unwrap()
}
#[test]
fn public_audio_edit_commands_resolve_the_registered_definition() {
    let owner = pm_kernel::Writer {
        realm: 101,
        generation: 1,
    };
    let definition = json!({"id":"test.gain","label":"Gain","group":"Audio","params":[{"type":"number","k":"gain","label":"Gain","unit":"dB","min":-96,"max":24,"def":0}],"graph":[{"type":"gain","gainDb":{"param":"gain"}}]});
    let (state, _) = StateSnapshot::default()
        .stage(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::Register {
                registry: "audioEffects".into(),
                entry: Registration {
                    token: "audio-test".into(),
                    owner,
                    id: "test.gain".into(),
                    definition: model(&definition),
                },
            }],
        })
        .unwrap();
    let mut store = DocumentStore::new_untitled(model(
        &json!({"id":"P","compId":"C","w":320,"h":180,"fps":30,"dur":1,"layers":[{"id":"sound","type":"audio","from":0,"dur":1,"d":{"asset":"s"}}],"comps":{},"assets":{}}),
    ));
    let batch = pm_mcp::commands::plan_with_definitions(
        &document(&store),
        &[],
        &[json!({"type":"add_audio_effect","target":"sound","effect":"test.gain"})],
        0.0,
        Some(&state),
    )
    .unwrap();
    let id = batch.results[0]["data"]["effectId"]
        .as_str()
        .unwrap()
        .to_owned();
    for command in batch.commands {
        store.execute(command, Origin::Interface).unwrap();
    }
    for command in [
        json!({"type":"set_audio_effect_parameter","target":"sound","effect":id,"key":"gain","value":-12}),
        json!({"type":"enable_audio_effect","target":"sound","effect":id,"enabled":false}),
        json!({"type":"reorder_audio_effect","target":"sound","effect":id,"index":0}),
        json!({"type":"remove_audio_effect","target":"sound","effect":id}),
    ] {
        let batch = pm_mcp::commands::plan_with_definitions(
            &document(&store),
            &[],
            &[command],
            0.0,
            Some(&state),
        )
        .unwrap();
        for command in batch.commands {
            store.execute(command, Origin::Interface).unwrap();
        }
    }
    let loc = pm_kernel::model::find_layer(store.root().unwrap(), "sound").unwrap();
    let layer = pm_kernel::model::layer_at(store.root().unwrap(), &loc)
        .unwrap()
        .to_value();
    assert!(layer.get("audioFx").unwrap().as_array().unwrap().is_empty());
}
