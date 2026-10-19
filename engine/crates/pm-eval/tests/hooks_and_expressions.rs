//! The expression interface and the hooks later lanes extend.

use std::sync::Arc;

use pm_eval::evaluator::Content;
use pm_eval::{
    Comp, CompiledExpression, EvalHooks, Evaluator, ExprContext, ExprError, ExpressionEngine,
    JsResult, JsValue, PropAddress, PropEval,
};
use pm_model::json::parse;
use pm_model::{Document, JsString, Node};

const DOC: &str = r##"{
  "id": "P", "name": "Project", "compId": "C", "params": { "gain": { "value": 4 } },
  "comps": {
    "C": { "id": "C", "name": "Main", "fps": 29.97, "dur": 10, "layers": [
      { "id": "A", "type": "solid", "from": 1, "dur": 5, "on": true,
        "p": { "opacity": { "v": 50, "kf": [], "expr": "boom" },
               "rotation": { "v": 0, "kf": [ { "t": 0, "v": 0 }, { "t": 2, "v": 20 } ], "expr": "silent" },
               "skew": { "v": 1, "kf": [], "expr": "frame" },
               "scale.x": { "v": 1, "kf": [], "expr": "gain" } },
        "d": { "w": { "v": 10, "kf": [], "expr": null } } } ] },
    "N": { "id": "N", "fps": 25, "dur": 4, "params": { "gain": { "value": 9 } }, "layers": [
      { "id": "B", "type": "solid", "from": 0, "dur": 4, "on": true, "p": { "scale.x": { "v": 1, "kf": [], "expr": "gain" } } } ] }
  }
}"##;

struct Engine;
struct Expr(&'static str);

impl ExpressionEngine for Engine {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        ["boom", "silent", "frame", "gain"]
            .into_iter()
            .find(|k| source.as_str() == Some(k))
            .map(|k| Arc::new(Expr(k)) as Arc<dyn CompiledExpression>)
    }
}

impl CompiledExpression for Expr {
    fn needs_idx(&self) -> bool {
        false
    }

    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
        match self.0 {
            "boom" => Err(ExprError {
                message: Some("boom".into()),
            }),
            "silent" => Err(ExprError { message: None }),
            "frame" => Ok(JsValue::Number(cx.frame.map_or(f64::NAN, |f| f as f64))),
            "gain" => cx.param(&JsString::from("gain")).map_err(|e| ExprError {
                message: Some(e.message),
            }),
            _ => unreachable!(),
        }
    }
}

fn layer<'d>(cx: &Comp<'d>) -> &'d Node {
    &cx.layers()[0]
}

#[test]
fn expression_errors_and_context() {
    let doc = Document::new(Some(parse(DOC).unwrap()), 1);
    let cx = Comp::resolve(&doc, "C").unwrap();
    let mut ev = Evaluator::new().with_expressions(Arc::new(Engine));
    let l = Some(layer(&cx));

    // A thrown error keeps the channel's value and records the message.
    assert_eq!(
        ev.ev(&cx, l, &"opacity".into(), 2.0).unwrap(),
        JsValue::Number(50.0)
    );
    assert_eq!(ev.expression_error("C", "A", "opacity"), Some("boom"));
    // A thrown non-Error records the generic text; the keyframed value stays.
    assert_eq!(
        ev.ev(&cx, l, &"rotation".into(), 2.0).unwrap(),
        JsValue::Number(10.0)
    );
    assert_eq!(
        ev.expression_error("C", "A", "rotation"),
        Some("Expression failed.")
    );
    // `frame` is the composition frame at the exact rate (section 5, rule 9):
    // 29.97 snaps to 30000/1001, and 2 s is inside frame 59.
    assert_eq!(
        ev.ev(&cx, l, &"skew".into(), 2.0).unwrap(),
        JsValue::Number(59.0)
    );
    assert_eq!(
        ev.ev(&cx, l, &"skew".into(), 60.0 / 29.97).unwrap(),
        JsValue::Number(60.0)
    );
    // The open composition reads project parameters (the legacy `PM.proj`).
    assert_eq!(
        ev.ev(&cx, l, &"scale.x".into(), 2.0).unwrap(),
        JsValue::Number(4.0)
    );
    // A nested composition reads its own.
    let nested = Comp::resolve(&doc, "N").unwrap();
    assert_eq!(
        ev.ev(&nested, Some(layer(&nested)), &"scale.x".into(), 1.0)
            .unwrap(),
        JsValue::Number(9.0)
    );
    assert_eq!(nested.expression_fps(), JsValue::Number(25.0));
    assert_eq!(cx.project_fps(), JsValue::Number(29.97));

    // Without an engine every expression is invalid syntax.
    let mut bare = Evaluator::new();
    assert_eq!(
        bare.ev(&cx, l, &"opacity".into(), 2.0).unwrap(),
        JsValue::Number(50.0)
    );
    assert_eq!(
        bare.expression_error("C", "A", "opacity"),
        Some("Expression syntax is invalid.")
    );

    // Ticks on the composition grid map to the document's own floats.
    let rate = cx.rate().unwrap();
    let ticks = rate.rate.frame_start(60).unwrap();
    assert_eq!(cx.seconds_at(ticks), 60.0 / 29.97);
    let id = ev.intern(PropAddress::channel("C", "A", "skew"));
    assert_eq!(
        ev.ev_id_ticks(&doc, id, ticks).unwrap(),
        JsValue::Number(60.0)
    );
    assert_eq!(
        ev.ev_id(&doc, id, 60.0 / 29.97).unwrap(),
        JsValue::Number(60.0)
    );
}

struct Hooks;

impl EvalHooks for Hooks {
    fn overrides_channels(&self) -> bool {
        true
    }

    fn channel(
        &self,
        _cx: &Comp<'_>,
        _layer: Option<&Node>,
        key: &JsString,
        time: f64,
    ) -> Option<JsResult<JsValue>> {
        (key.as_str() == Some("opacity")).then_some(Ok(JsValue::Number(time * 100.0)))
    }

    fn active(
        &self,
        _cx: &Comp<'_>,
        _layer: Option<&Node>,
        time: f64,
        base: bool,
        _eval: &mut dyn PropEval,
    ) -> JsResult<bool> {
        Ok(base && time < 3.0)
    }

    fn resolve_content(
        &self,
        cx: &Comp<'_>,
        layer: &Node,
        time: f64,
        content: &mut Content,
        eval: &mut dyn PropEval,
    ) -> JsResult<()> {
        let w = layer.get("d").and_then(|d| d.get("w"));
        let value = eval.ev_p(cx, Some(layer), w, time, &"c.w".into())?;
        content.insert(JsString::from("extra"), value);
        Ok(())
    }
}

#[test]
fn hooks_extend_ev_active_and_content() {
    let doc = Document::new(Some(parse(DOC).unwrap()), 1);
    let cx = Comp::resolve(&doc, "C").unwrap();
    let mut ev = Evaluator::new().with_hooks(Arc::new(Hooks));
    let l = layer(&cx);
    assert_eq!(
        ev.ev(&cx, Some(l), &"opacity".into(), 0.5).unwrap(),
        JsValue::Number(50.0)
    );
    let id = ev.intern(PropAddress::channel("C", "A", "opacity"));
    assert_eq!(ev.ev_id(&doc, id, 0.25).unwrap(), JsValue::Number(25.0));
    assert!(ev.active(&cx, Some(l), 2.0).unwrap());
    assert!(
        !ev.active(&cx, Some(l), 4.0).unwrap(),
        "the hook narrows the base answer"
    );
    let content = ev.resolve_content(&cx, l, 2.0).unwrap();
    assert_eq!(
        content.get(&JsString::from("w")),
        Some(&JsValue::Number(10.0))
    );
    assert_eq!(
        content.get(&JsString::from("extra")),
        Some(&JsValue::Number(10.0))
    );
}

#[test]
fn resident_expression_plans_do_not_recompile_more_than_256_unique_sources_each_frame() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counting(Arc<AtomicUsize>);
    struct Identity;
    impl ExpressionEngine for Counting {
        fn compile(&self, _: &JsString) -> Option<Arc<dyn CompiledExpression>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Some(Arc::new(Identity))
        }
    }
    impl CompiledExpression for Identity {
        fn needs_idx(&self) -> bool {
            false
        }
        fn needs_frame(&self) -> bool {
            false
        }
        fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
            Ok(cx.value.clone())
        }
    }
    let layers = (0..400).map(|i| format!(r#"{{"id":"L{i}","from":0,"p":{{"opacity":{{"v":4,"kf":[{{"t":0,"v":0}},{{"t":2,"v":20}}],"expr":"source-{i}"}}}}}}"#)).collect::<Vec<_>>().join(",");
    let json = format!(
        r#"{{"id":"P","compId":"C","comps":{{"C":{{"id":"C","fps":30,"layers":[{layers}]}}}}}}"#
    );
    let doc = Document::new(Some(parse(&json).unwrap()), 1);
    let count = Arc::new(AtomicUsize::new(0));
    let mut ev = Evaluator::new().with_expressions(Arc::new(Counting(count.clone())));
    let ids = (0..400)
        .map(|i| ev.intern(PropAddress::channel("C", &format!("L{i}"), "opacity")))
        .collect::<Vec<_>>();
    let mut values = vec![0.0; ids.len()];
    let mut kinds = vec![pm_eval::ValueKind::Number; ids.len()];
    for time in [0.0, 1.0, 2.0, 1.0] {
        ev.ev_batch(&doc, &ids, time, &mut values, &mut kinds);
        assert!(values.iter().all(|n| *n == time * 10.0));
        assert_eq!(count.load(Ordering::Relaxed), 400);
    }
}

#[test]
fn root_edits_keep_sources_and_only_refresh_changed_bindings() {
    use pm_model::{Patch, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Source(Arc<AtomicUsize>);
    struct Fixed {
        gain: f64,
    }
    impl ExpressionEngine for Source {
        fn compile(&self, _: &JsString) -> Option<Arc<dyn CompiledExpression>> {
            Some(Arc::new(Source(self.0.clone())))
        }
    }
    impl CompiledExpression for Source {
        fn needs_idx(&self) -> bool {
            false
        }
        fn bind(&self, cx: &ExprContext<'_>) -> Option<Arc<dyn CompiledExpression>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Some(Arc::new(Fixed {
                gain: cx.param(&"gain".into()).unwrap().to_number(),
            }))
        }
        fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
            Ok(cx.value.clone())
        }
    }
    impl CompiledExpression for Fixed {
        fn needs_idx(&self) -> bool {
            false
        }
        fn binding_matches(&self, cx: &ExprContext<'_>) -> Option<bool> {
            Some(self.gain.to_bits() == cx.param(&"gain".into()).unwrap().to_number().to_bits())
        }
        fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
            Ok(JsValue::Number(self.gain + cx.t))
        }
    }
    let mut doc = Document::new(Some(parse(DOC).unwrap()), 1);
    let count = Arc::new(AtomicUsize::new(0));
    let mut ev = Evaluator::new().with_expressions(Arc::new(Source(count.clone())));
    let id = ev.intern(PropAddress::channel("C", "A", "scale.x"));
    assert_eq!(ev.ev_id(&doc, id, 2.0).unwrap(), JsValue::Number(5.0));
    assert_eq!(count.load(Ordering::Relaxed), 1);
    let plans = ev.cache_stats().plans;
    let set = |path: Vec<pm_model::PathPart>, value| Patch {
        path,
        exists: true,
        value: Some(value),
    };
    doc.apply(&[set(vec!["name".into()], Value::from("Unrelated"))])
        .unwrap();
    assert_eq!(ev.ev_id(&doc, id, 2.0).unwrap(), JsValue::Number(5.0));
    assert_eq!(ev.cache_stats().plans, plans);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    doc.apply(&[set(
        vec!["params".into(), "gain".into(), "value".into()],
        Value::Number(9.0),
    )])
    .unwrap();
    assert_eq!(ev.ev_id(&doc, id, 2.0).unwrap(), JsValue::Number(10.0));
    assert_eq!(ev.cache_stats().plans, plans + 1);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    // The binding remains valid while local time changes independently.
    doc.apply(&[set(
        vec![
            "comps".into(),
            "C".into(),
            "layers".into(),
            0usize.into(),
            "from".into(),
        ],
        Value::Number(3.0),
    )])
    .unwrap();
    assert_eq!(ev.ev_id(&doc, id, 2.0).unwrap(), JsValue::Number(8.0));
    assert_eq!(count.load(Ordering::Relaxed), 2);
}
