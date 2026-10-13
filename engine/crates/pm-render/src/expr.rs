//! The [`ExpressionEngine`] the renderer evaluates with: `pm-expr` (legacy
//! dialect) reading the layer and composition through `pm_expr::JsonHost`
//! over owned copies. It is the adapter `pm-eval`'s corpus comparison
//! validates against the legacy evaluator (`tests/support/pm_expr_engine.rs`
//! there); the production integration of `pm-expr` with the evaluator will
//! replace both.

use std::sync::Arc;

use pm_eval::{CompiledExpression, ExprContext, ExprError, ExpressionEngine, JsValue};
use pm_expr::{Context, Dialect, Expression, JsStr, JsonHost, Value};
use pm_model::JsString;

/// `pm-expr` in the legacy dialect.
#[derive(Debug, Default, Clone, Copy)]
pub struct PmExprEngine;

struct Compiled(Expression);

impl ExpressionEngine for PmExprEngine {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        let units: Vec<u16> = source.utf16_units().collect();
        Expression::compile_utf16(&units, Dialect::Legacy)
            .map(|e| Arc::new(Compiled(e)) as Arc<dyn CompiledExpression>)
    }
}

fn js_str(s: &JsString) -> JsStr {
    JsStr::from_units(s.utf16_units().collect::<Vec<u16>>())
}

impl CompiledExpression for Compiled {
    fn needs_idx(&self) -> bool {
        self.0.needs_idx()
    }

    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
        let layer = cx.layer.map(|n| n.to_value());
        let comp = cx.comp.node().to_value();
        let boxed;
        let value = match cx.value {
            JsValue::Undefined => Value::Undefined,
            JsValue::Null => Value::Null,
            JsValue::Bool(b) => Value::Bool(*b),
            JsValue::Number(n) => Value::Number(*n),
            JsValue::String(s) => Value::String(js_str(s)),
            JsValue::Object(v) => {
                boxed = (**v).clone();
                JsonHost::value(&boxed)
            }
        };
        let ctx = Context {
            t: cx.t,
            time: cx.time,
            fps: cx.fps.to_number(),
            frame: cx.frame.map_or(f64::NAN, |f| f as f64),
            value,
            layer: layer.as_ref().map_or(Value::Undefined, JsonHost::value),
            comp: JsonHost::value(&comp),
            channel: js_str(cx.channel),
            idx: cx.idx,
        };
        let out = self.0.evaluate(&JsonHost::new(), &ctx);
        match out.result {
            Ok(Value::Number(n)) => Ok(JsValue::Number(n)),
            Ok(Value::Bool(b)) => Ok(JsValue::Bool(b)),
            Ok(Value::String(s)) => Ok(JsValue::String(JsString::from_utf16_units(s.units()))),
            Ok(Value::Null) => Ok(JsValue::Null),
            Ok(_) => Ok(JsValue::Undefined),
            Err(e) => Err(ExprError {
                message: Some(e.message()),
            }),
        }
    }
}
