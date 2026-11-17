//! Pixel regression: parameter-driven transforms equal literal transforms,
//! including nested precomps and edits to the parameter's current value.
#[path = "support/doc.rs"]
mod doc;

use pm_model::Document;
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer};
use serde_json::{Value, json};

fn project(amount: f64, literal: bool, nested: bool) -> Value {
    let mut layer = doc::layer(
        "solid",
        "solid",
        json!({
            "w":8,"h":8,"color":"#FFFFFF","fill":doc::solid_fill("#FFFFFF")
        }),
        16.0,
        16.0,
        10.0,
    );
    layer["threeD"] = json!(true);
    doc::set(&mut layer, "anchor.x", json!(4));
    doc::set(&mut layer, "anchor.y", json!(4));
    doc::set(
        &mut layer,
        "rotation.y",
        if literal {
            doc::ch(json!(amount * 40.0))
        } else {
            json!({"v":0,"expr":"value + param('amount') * 40"})
        },
    );
    let precomp = doc::layer(
        "precomp",
        "precomp",
        json!({"comp":"inner"}),
        0.0,
        0.0,
        10.0,
    );
    let mut project = doc::project(
        32,
        32,
        10.0,
        doc::no_fill(),
        vec![if nested { precomp } else { layer.clone() }],
    );
    project["params"] = json!({"amount":{"value":if nested { 0.25 } else { amount }}});
    if nested {
        doc::composition(&mut project, "inner", 32, 32, 10.0, vec![layer]);
        project["comps"]["inner"]["params"] = json!({"amount":{"value":amount}});
        project["comps"]["inner"]["backgroundFill"] = doc::no_fill();
    }
    project
}

fn model(project: Value, canonical: bool, revision: u64) -> Document {
    let v = pm_model::json::parse(&project.to_string()).unwrap();
    Document::new(
        Some(if canonical {
            pm_model::canonical::canonical_project(v)
        } else {
            v
        }),
        revision,
    )
}

#[test]
fn parameter_edits_update_preview_and_final_pixels_in_root_and_precomp() {
    let mut renderer = Renderer::headless().expect("headless Metal device");
    let mut literal_renderer = Renderer::headless().expect("headless reference device");
    let assets = MapResolver::default();
    for canonical in [false, true] {
        for nested in [false, true] {
            let mut previous = None;
            let mut document = model(project(0.0, false, nested), canonical, 1);
            for (index, amount) in [0.0, 1.0, 2.0].into_iter().enumerate() {
                let path = if nested {
                    json!(["comps", "inner", "params", "amount", "value"])
                } else {
                    json!(["params", "amount", "value"])
                };
                let patches = pm_model::patch::patches_from_value(
                    &pm_model::json::parse(
                        &json!([{"path":path,"exists":true,"value":amount}]).to_string(),
                    )
                    .unwrap(),
                )
                .unwrap();
                document.apply(&patches).unwrap();
                let literal = model(project(amount, true, nested), canonical, index as u64 + 1);
                for kind in [OutputKind::Preview, OutputKind::Final] {
                    let mut request = RenderRequest::sized(Size::new(32, 32), Precision::Float16);
                    request.background = false;
                    request.output_kind = kind;
                    let frame = renderer
                        .render(
                            &document,
                            "Ptest-root",
                            Ticks::new(0).unwrap(),
                            &request,
                            &assets,
                        )
                        .unwrap();
                    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                    let actual = renderer.read_rgba(&frame).unwrap();
                    assert!(actual.iter().any(|v| *v > 0.5), "fixture must draw pixels");
                    let expected_frame = literal_renderer
                        .render(
                            &literal,
                            "Ptest-root",
                            Ticks::new(0).unwrap(),
                            &request,
                            &assets,
                        )
                        .unwrap();
                    let expected = literal_renderer.read_rgba(&expected_frame).unwrap();
                    assert_eq!(
                        actual, expected,
                        "canonical={canonical}, nested={nested}, amount={amount}, {kind:?}"
                    );
                    if kind == OutputKind::Preview {
                        if let Some(old) = previous.replace(actual.clone()) {
                            assert_ne!(actual, old, "parameter edit must change frame");
                        }
                    }
                }
            }
        }
    }
}
