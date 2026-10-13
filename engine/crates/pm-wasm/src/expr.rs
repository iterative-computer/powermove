//! The production [`ExpressionEngine`]: `pm-expr` (legacy dialect, exactly
//! `expression.ts`) reading the mirror's nodes in place.
//!
//! Expressions see `layer`, `comp`, and `value` as the legacy evaluator hands
//! them over: the live layer object, `PM.curComp()` (for the open
//! composition, the stored project whose top level mixes project and
//! composition fields, which [`Comp::get`] reproduces), and the keyframed
//! value. Nothing is copied: [`NodeHost`] answers member reads from the
//! revisioned tree, and object identity (`comp.layers[0] === layer`) is node
//! identity.

use std::sync::Arc;

use pm_eval::{Comp, CompiledExpression, ExprContext, ExprError, ExpressionEngine, JsValue};
use pm_expr::{Context, Dialect, Expression, Host, JsStr, Object, Shape, Value};
use pm_model::{JsString, Node, NodeKind};

/// Compiles with `pm-expr`'s legacy dialect.
pub struct PmExpressions;

impl ExpressionEngine for PmExpressions {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        let units: Vec<u16> = source.utf16_units().collect();
        Expression::compile_utf16(&units, Dialect::Legacy)
            .map(|e| Arc::new(Compiled(e)) as Arc<dyn CompiledExpression>)
    }
}

struct Compiled(Expression);

/// A document object as an expression sees it.
#[derive(Clone, Copy, Debug)]
pub enum Obj<'a> {
    /// The legacy composition object (`PM.curComp()`).
    Comp(&'a Comp<'a>),
    /// A node of the mirror.
    Node(&'a Node),
    /// An object or array inside a computed value (`value` of a channel whose
    /// keys hold objects).
    Value(&'a pm_model::Value),
}

impl PartialEq for Obj<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Obj::Comp(a), Obj::Comp(b)) => std::ptr::eq(a.node(), b.node()),
            (Obj::Node(a), Obj::Node(b)) => std::ptr::eq(*a, *b),
            (Obj::Value(a), Obj::Value(b)) => std::ptr::eq(*a, *b),
            // The open composition's legacy object is the project root;
            // `comp` is never reachable as a plain node, so these differ.
            _ => false,
        }
    }
}

fn js_str(s: &JsString) -> JsStr {
    JsStr::from_units(s.utf16_units().collect::<Vec<u16>>())
}

/// Reads the mirror's nodes (and computed values) for `pm-expr`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeHost<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> NodeHost<'a> {
    pub fn node(node: &'a Node) -> Value<Obj<'a>> {
        match node.kind() {
            NodeKind::Null => Value::Null,
            NodeKind::Bool(b) => Value::Bool(*b),
            NodeKind::Number(n) => Value::Number(*n),
            NodeKind::String(s) => Value::String(js_str(s)),
            NodeKind::Array(_) | NodeKind::Object(_) => Value::Object(Object::Doc(Obj::Node(node))),
        }
    }

    pub fn value(value: &'a pm_model::Value) -> Value<Obj<'a>> {
        match value {
            pm_model::Value::Null => Value::Null,
            pm_model::Value::Bool(b) => Value::Bool(*b),
            pm_model::Value::Number(n) => Value::Number(*n),
            pm_model::Value::String(s) => Value::String(js_str(s)),
            pm_model::Value::Array(_) | pm_model::Value::Object(_) => {
                Value::Object(Object::Doc(Obj::Value(value)))
            }
        }
    }

    fn js(value: &'a JsValue) -> Value<Obj<'a>> {
        match value {
            JsValue::Undefined => Value::Undefined,
            JsValue::Null => Value::Null,
            JsValue::Bool(b) => Value::Bool(*b),
            JsValue::Number(n) => Value::Number(*n),
            JsValue::String(s) => Value::String(js_str(s)),
            JsValue::Object(v) => NodeHost::value(v),
        }
    }
}

impl<'a> Host for NodeHost<'a> {
    type Obj = Obj<'a>;

    fn shape(&self, obj: &Obj<'a>) -> Shape {
        match obj {
            Obj::Comp(_) => Shape::Object,
            Obj::Node(n) => match n.kind() {
                NodeKind::Array(items) => Shape::Array { len: items.len() },
                _ => Shape::Object,
            },
            Obj::Value(v) => match v {
                pm_model::Value::Array(items) => Shape::Array { len: items.len() },
                _ => Shape::Object,
            },
        }
    }

    fn element(&self, array: &Obj<'a>, index: usize) -> Value<Obj<'a>> {
        match array {
            Obj::Node(n) => match n.kind() {
                NodeKind::Array(items) => items.get(index).map_or(Value::Undefined, NodeHost::node),
                _ => Value::Undefined,
            },
            Obj::Value(pm_model::Value::Array(items)) => {
                items.get(index).map_or(Value::Undefined, NodeHost::value)
            }
            _ => Value::Undefined,
        }
    }

    fn property(&self, object: &Obj<'a>, key: &JsStr) -> Option<Value<Obj<'a>>> {
        let key = JsString::from_utf16_units(key.units());
        match object {
            Obj::Comp(comp) => match comp.get(key.as_bytes()) {
                pm_eval::node::Got::Node(n) => Some(NodeHost::node(n)),
                _ => None,
            },
            Obj::Node(n) => match n.kind() {
                NodeKind::Object(map) => map.get(&key).map(NodeHost::node),
                _ => None,
            },
            Obj::Value(pm_model::Value::Object(map)) => map.get_key(&key).map(NodeHost::value),
            Obj::Value(_) => None,
        }
    }
}

impl CompiledExpression for Compiled {
    fn needs_idx(&self) -> bool {
        self.0.needs_idx()
    }

    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
        let host = NodeHost(std::marker::PhantomData);
        let ctx = Context {
            t: cx.t,
            time: cx.time,
            fps: cx.fps.to_number(),
            frame: cx.frame.map_or(f64::NAN, |f| f as f64),
            value: NodeHost::js(cx.value),
            layer: cx.layer.map_or(Value::Undefined, NodeHost::node),
            comp: Value::Object(Object::Doc(Obj::Comp(cx.comp))),
            channel: js_str(cx.channel),
            idx: cx.idx,
        };
        let out = self.0.evaluate(&host, &ctx);
        match out.result {
            Ok(Value::Number(n)) => Ok(JsValue::Number(n)),
            Ok(Value::Bool(b)) => Ok(JsValue::Bool(b)),
            Ok(Value::String(s)) => Ok(JsValue::String(JsString::from_utf16_units(s.units()))),
            Ok(Value::Null) => Ok(JsValue::Null),
            // Objects and `undefined` are rejected by `evalProp` alike
            // ("did not return a finite value"); the evaluator only checks
            // for a finite number, a string, or a boolean.
            Ok(_) => Ok(JsValue::Undefined),
            Err(e) => Err(ExprError { message: Some(e.message()) }),
        }
    }
}
