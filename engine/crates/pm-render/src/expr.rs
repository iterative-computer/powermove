//! The renderer uses the kernel's expression host so both read the same
//! composition view, including project parameters on canonical open comps.

use pm_eval::{CompiledExpression, ExpressionEngine};
use pm_model::JsString;
use std::sync::Arc;

/// Renderer entry point for the shared legacy expression evaluator.
#[derive(Debug, Default, Clone, Copy)]
pub struct PmExprEngine;

impl ExpressionEngine for PmExprEngine {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        pm_wasm::expr::PmExpressions.compile(source)
    }
}

#[cfg(test)]
mod parameter_tests {
    use super::*;
    use pm_eval::{Comp, Evaluator, JsValue, Scope};
    use pm_model::Document;
    use serde_json::json;

    fn document(canonical: bool) -> Document {
        let layer = json!({
            "id":"layer", "type":"solid", "from":0, "dur":10,
            "p":{
                "rotation.y":{"v":0,"expr":"value + param('amount') * 40"},
                "position.x":{"v":0,"expr":"param('amount') * 40"},
                "scale.x":{"v":100}, "scale.y":{"v":100}
            }
        });
        let mut stored = json!({
            "id":"project", "compId":"outer", "fps":30,
            "params":{"amount":{"value":1}}, "layers":[layer.clone()],
            "comps":{"inner":{"id":"inner","fps":30,
                "params":{"amount":{"value":3}},"layers":[layer]}}
        });
        // An actual precomp reference makes the ownership explicit.
        stored["layers"].as_array_mut().unwrap().push(json!({
            "id":"precomp", "type":"precomp", "from":0, "dur":10, "d":{"comp":"inner"}
        }));
        let value = pm_model::json::parse(&stored.to_string()).unwrap();
        Document::new(
            Some(if canonical {
                pm_model::canonical::canonical_project(value)
            } else {
                value
            }),
            1,
        )
    }

    fn compare(
        doc: &Document,
        id: &str,
        time: f64,
        expected: f64,
        render: &mut Evaluator,
        kernel: &mut Evaluator,
    ) {
        let comp = Comp::resolve(doc, id).unwrap();
        let layer = comp.node().get("layers").unwrap().at(0).unwrap();
        let key = JsString::from("rotation.y");
        let actual = render.ev(&comp, Some(layer), &key, time).unwrap();
        assert_eq!(actual, kernel.ev(&comp, Some(layer), &key, time).unwrap());
        assert_eq!(actual, JsValue::Number(expected), "{id} at {time}");
        // This is the transform path consumed by the render planner.
        let matrix = Scope::open(render, &comp)
            .world_matrix(layer, time)
            .unwrap();
        assert_eq!(matrix[4], expected, "rendered translation for {id}");
    }

    fn evaluators() -> (Evaluator, Evaluator) {
        (
            Evaluator::new().with_expressions(Arc::new(PmExprEngine)),
            Evaluator::new().with_expressions(Arc::new(pm_wasm::expr::PmExpressions)),
        )
    }

    #[test]
    fn root_nested_and_missing_parameters_match_v2_and_kernel() {
        for canonical in [false, true] {
            let mut doc = document(canonical);
            let (mut render, mut kernel) = evaluators();
            compare(&doc, "outer", 0.0, 40.0, &mut render, &mut kernel);
            compare(&doc, "inner", 0.0, 120.0, &mut render, &mut kernel);
            // V2 rootComp deliberately excludes project params.
            compare(&doc, "@root:outer", 0.0, 0.0, &mut render, &mut kernel);
            let patches = pm_model::patch::patches_from_value(
                &pm_model::json::parse(
                    r#"[{"exists":false,"path":["comps","inner","params","amount"]}]"#,
                )
                .unwrap(),
            )
            .unwrap();
            doc.apply(&patches).unwrap();
            // Missing inner params must not inherit the outer value.
            compare(&doc, "inner", 0.0, 0.0, &mut render, &mut kernel);
            let patches = pm_model::patch::patches_from_value(
                &pm_model::json::parse(r#"[{"exists":false,"path":["params"]}]"#).unwrap(),
            )
            .unwrap();
            doc.apply(&patches).unwrap();
            compare(&doc, "outer", 0.0, 0.0, &mut render, &mut kernel);
        }
    }

    #[test]
    fn changing_parameter_value_updates_rendered_transform() {
        for canonical in [false, true] {
            let mut doc = document(canonical);
            let (mut render, mut kernel) = evaluators();
            for (time, value) in [(0.0, 1.0), (1.0, 2.0), (2.0, 0.0)] {
                let patches = pm_model::patch::patches_from_value(
                    &pm_model::json::parse(
                        &json!([{"exists":true,"path":["params","amount","value"],"value":value}])
                            .to_string(),
                    )
                    .unwrap(),
                )
                .unwrap();
                doc.apply(&patches).unwrap();
                render.open_channel_memo(&doc);
                compare(&doc, "outer", time, value * 40.0, &mut render, &mut kernel);
                render.close_channel_memo();
            }
        }
    }

    #[test]
    fn legacy_expression_safety_limits_are_preserved() {
        for source in ["const x = 1; x", "(() => 1)()"] {
            assert!(PmExprEngine.compile(&JsString::from(source)).is_none());
        }
        let too_many_nodes = std::iter::repeat_n("1", 100).collect::<Vec<_>>().join("+");
        assert!(
            PmExprEngine
                .compile(&JsString::from(too_many_nodes.as_str()))
                .is_none()
        );
        assert_eq!(pm_expr::MAX_SOURCE_CHARS, 2000);
        assert_eq!(pm_expr::MAX_NODES, 160);
        assert_eq!(pm_expr::MAX_DEPTH, 24);
        assert_eq!(pm_expr::MAX_CALL_ARGS, 12);
    }
}
