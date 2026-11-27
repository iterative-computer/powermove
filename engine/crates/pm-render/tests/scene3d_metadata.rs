//! Diagnostics travel with the scene that supplied the frame, including
//! cached draws and recoveries. The renderer does not re-evaluate a scene
//! merely to obtain its UI metadata.
mod support;

use pm_protocol::{color::Precision, common::Size};
use pm_render::{Frame, MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::{doc::*, ticks};

fn scene() -> Value {
    let mut object = object3d("object", "box", json!({}), "#FFFFFF", 160, 90, 2.0);
    object["d"]["data"]["object"]["material"]["maps"] = json!({"color":"offline"});
    project(
        160,
        90,
        2.0,
        solid_fill("#000000"),
        vec![camera("camera", [0.0, 0.0, 4.0], false, 2.0), object],
    )
}
fn render(renderer: &mut Renderer, value: &Value) -> Frame {
    renderer
        .render(
            &load(value),
            "Ptest-root",
            ticks(0.0),
            &RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16),
            &MapResolver::default(),
        )
        .unwrap()
}

#[test]
fn scene_warnings_are_grouped_and_clear_when_the_scene_recovers() {
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_draw_reuse(true);
    let mut value = scene();
    let mut second = value["layers"][1].clone();
    second["id"] = json!("second");
    value["layers"].as_array_mut().unwrap().push(second);
    for _ in 0..2 {
        let frame = render(&mut renderer, &value);
        assert!(frame.failures.is_empty());
        let [report] = frame.scene3d_meta.as_slice() else {
            panic!("{:?}", frame.scene3d_meta)
        };
        assert_eq!(report.composition.as_str(), "Ptest-root");
        assert_eq!(
            report.layers.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
            ["object", "second"]
        );
        assert_eq!(
            report.warnings,
            ["Missing texture: offline", "Missing texture: offline"]
        );
        assert!(report.error.is_none());
    }
    for object in &mut value["layers"].as_array_mut().unwrap()[1..] {
        object["d"]["data"]["object"]["material"]["maps"] = json!({});
    }
    let frame = render(&mut renderer, &value);
    let [report] = frame.scene3d_meta.as_slice() else {
        panic!("{:?}", frame.scene3d_meta)
    };
    assert!(report.warnings.is_empty());
    assert!(report.error.is_none());
}

#[test]
fn missing_models_warn_and_corrupt_composition_scenes_report_every_object() {
    let mut renderer = Renderer::headless().unwrap();
    let mut value = scene();
    value["layers"][1]["d"]["data"]["object"]["source"] = json!({"assetId":"absent"});
    value["layers"][1]["d"]["data"]["object"]["material"]["maps"] = json!({});
    let frame = render(&mut renderer, &value);
    assert!(frame.failures.is_empty());
    assert_eq!(frame.scene3d_meta[0].warnings, ["Missing model: absent"]);
    value["layers"][1]["d"]["data"]["object"]["source"] = json!({"primitive":"knot"});
    let frame = render(&mut renderer, &value);
    assert_eq!(frame.failures.len(), 1);
    assert_eq!(frame.scene3d_meta[0].layers[0].as_str(), "object");
    assert!(frame.scene3d_meta[0].warnings.is_empty());
    assert!(
        frame.scene3d_meta[0]
            .error
            .as_ref()
            .unwrap()
            .contains("Invalid 3D scene")
    );
}

#[test]
fn corrupt_legacy_scene_checkerboard_carries_its_missing_metadata() {
    let captured: Value = serde_json::from_str(include_str!(
        "../../pm-model/tests/fixtures/corpus/3d-legacy-scene.json"
    ))
    .unwrap();
    let mut legacy = layer(
        "legacy",
        "extension",
        captured["layers"][0]["d"].clone(),
        0.0,
        0.0,
        2.0,
    );
    legacy["d"]["data"]["scene"]["foreign"] = json!(true);
    let value = project(160, 90, 2.0, solid_fill("#000000"), vec![legacy]);
    let mut renderer = Renderer::headless().unwrap();
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .unwrap();
    let frame = render(&mut renderer, &value);
    let [report] = frame.scene3d_meta.as_slice() else {
        panic!("{:?}", frame.scene3d_meta)
    };
    assert_eq!(report.layers[0].as_str(), "legacy");
    assert!(report.error.as_ref().unwrap().contains("foreign"));
}
