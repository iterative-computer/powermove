use pm_kernel::{KernelActor, KernelConfig, OpenSource};
use pm_model::{Map, Value, json::parse};
use pm_panel_inspector::{EffectClipboard, Inspector};
#[test]
fn sixty_thousand_effect_parameters_evaluate_only_requested_window() {
    let mut project=parse(r#"{"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":10,"assets":{},"comps":{},"layers":[]}"#).unwrap();
    let mut parameters = Map::new();
    for i in 0..60_001 {
        parameters.insert(
            format!("p{i:05}").as_str(),
            parse(&format!(r#"{{"v":{i},"kf":[],"expr":null}}"#)).unwrap(),
        );
    }
    // A hidden expression would fail if the panel eagerly evaluated every row.
    parameters.insert(
        "p30000",
        parse(r#"{"v":0,"kf":[],"expr":"unknownFunction()"}"#).unwrap(),
    );
    let mut layer=parse(r#"{"id":"L","type":"shape","from":0,"dur":10,"on":true,"p":{},"d":{},"masks":[],"fx":[{"id":"F","type":"missing-but-preserved","on":true,"p":{}}]}"#).unwrap();
    layer
        .as_object_mut()
        .unwrap()
        .get_mut("fx")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap()
        .insert("p", Value::Object(parameters));
    project
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(layer);
    let actor = KernelActor::spawn(KernelConfig::default());
    let handle = actor.handle();
    handle
        .open("P", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let mut model = Inspector::new(handle.published("P").unwrap(), EffectClipboard::default());
    let window = model.property_window("L", 59_990..60_001, 2.0).unwrap();
    assert_eq!(window.total, 60_001);
    assert_eq!(window.rows.len(), 11);
    assert_eq!(window.rows[10].id.path, "F.p60000");
    assert_eq!(window.rows[10].state.value, 60_000.0.into());
    assert_eq!(
        model
            .property_window("L", 100..103, 3.0)
            .unwrap()
            .rows
            .len(),
        3
    );
    assert_eq!(
        model
            .property_window("L", usize::MAX..usize::MAX, 0.0)
            .unwrap()
            .range,
        60_001..60_001
    );
    actor.shutdown();
}
