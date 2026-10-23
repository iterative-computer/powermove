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

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use pm_eval::{Comp, CompiledExpression, ExprContext, ExprError, ExpressionEngine, JsValue};
use pm_expr::{Context, Dialect, Expression, GeneralBinding, Host, JsStr, Object, Shape, Value};
use pm_model::{JsString, Node, NodeKind};

/// Compiles with `pm-expr`'s legacy dialect.
pub struct PmExpressions;

impl ExpressionEngine for PmExpressions {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        let units: Vec<u16> = source.utf16_units().collect();
        Expression::compile_utf16(&units, Dialect::Legacy).map(|e| {
            let channel = e.needs_channel();
            let mut keys: Vec<_> = e
                .literal_strings()
                .map(|s| (s.clone(), JsString::from_utf16_units(s.units())))
                .collect();
            keys.extend(
                ["params", "value", "toString"].map(|s| (JsStr::from(s), JsString::from(s))),
            );
            // A source has a numeric plan or a general binding, never both.
            let uses_layer = e.numeric_fixed_uses_layer() || e.general_uses_layer();
            let layer_local = e.numeric_fixed_is_layer_local() || e.general_is_layer_local();
            let source = Arc::new(Source {
                expression: e,
                channel,
                keys,
                empty: JsStr::default(),
                uses_layer,
                layer_local,
                bindings: Mutex::new(BindingPool::default()),
                latest: Mutex::new(None),
            });
            Arc::new(Compiled {
                source,
                bound: None,
                binding_bits: None,
                general: None,
                index: None,
                verified: Mutex::new(None),
            }) as Arc<dyn CompiledExpression>
        })
    }
}

#[derive(Default)]
struct BindingPool {
    entries: HashMap<Arc<[u64]>, Weak<Compiled>>,
    order: VecDeque<Arc<[u64]>>,
}

struct Source {
    expression: Expression,
    channel: bool,
    keys: Vec<(JsStr, JsString)>,
    empty: JsStr,
    // Weak entries avoid a Source -> Compiled -> Source ownership cycle.
    bindings: Mutex<BindingPool>,
    uses_layer: bool,
    layer_local: bool,
    latest: Mutex<Option<(BindingStamp, Weak<Compiled>)>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BindingStamp {
    view: u64,
    layer: usize,
    idx: u64,
    fps: u64,
}

impl Source {
    fn stamp(&self, cx: &ExprContext<'_>) -> BindingStamp {
        BindingStamp {
            view: cx.comp.binding_token(),
            layer: if self.uses_layer {
                cx.layer.map_or(0, |n| n as *const Node as usize)
            } else {
                0
            },
            idx: if self.expression.needs_idx() {
                cx.idx.to_bits()
            } else {
                0
            },
            fps: cx.fps.to_number().to_bits(),
        }
    }
}

struct Compiled {
    source: Arc<Source>,
    bound: Option<Expression>,
    binding_bits: Option<Arc<[u64]>>,
    /// A general binding (partial evaluation); never shared between slots.
    general: Option<GeneralBinding>,
    index: Option<f64>,
    verified: Mutex<Option<(BindingStamp, bool)>>,
}

impl Compiled {
    fn expression(&self) -> &Expression {
        self.bound.as_ref().unwrap_or(&self.source.expression)
    }

    fn binding_context<'a>(&self, cx: &ExprContext<'a>) -> Context<Obj<'a>> {
        Context {
            t: 0.0,
            time: 0.0,
            fps: cx.fps.to_number(),
            frame: f64::NAN,
            value: Value::Undefined,
            layer: cx.layer.map_or(Value::Undefined, NodeHost::node),
            comp: Value::Object(Object::Doc(Obj::Comp(cx.comp))),
            channel: self.source.empty.clone(),
            idx: cx.idx,
        }
    }

    fn binding_bits(&self, cx: &ExprContext<'_>) -> Option<Vec<u64>> {
        let host = NodeHost(&self.source.keys, None);
        let ctx = self.binding_context(cx);
        let mut bits = self.source.expression.numeric_fixed_bits(&host, &ctx)?;
        // `needsIdx` is a source regex, including mentions inside literals.
        // Preserve the exact captured index for bytecode fallback too.
        if self.source.expression.needs_idx() {
            bits.push(cx.idx.to_bits());
        }
        Some(bits)
    }
}

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
    // Most document strings are ASCII: one exact allocation, no UTF-8 decode.
    let bytes = s.as_bytes();
    if bytes.is_ascii() {
        return JsStr::from_units(bytes.iter().map(|&b| u16::from(b)).collect::<Arc<[u16]>>());
    }
    JsStr::from_units(s.utf16_units().collect::<Vec<u16>>())
}

/// Reads the mirror's nodes (and computed values) for `pm-expr`, with the
/// channel's context for the `sample` and `motionPath` helpers.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeHost<'a>(&'a [(JsStr, JsString)], Option<&'a ExprContext<'a>>);

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

    fn sample(
        &self,
        layer: &JsStr,
        channel: &JsStr,
        local_time: f64,
    ) -> Option<Result<Value<Obj<'a>>, String>> {
        let cx = self.1?;
        let layer = JsString::from_utf16_units(layer.units());
        let channel = JsString::from_utf16_units(channel.units());
        Some(
            cx.sample(&layer, &channel, local_time)
                .map(|got| match got {
                    pm_eval::node::Got::Node(n) => NodeHost::node(n),
                    // A keyed object value (a path) is computed, not a node.
                    pm_eval::node::Got::Value(JsValue::Object(_)) => Value::Undefined,
                    pm_eval::node::Got::Value(v) => match v {
                        JsValue::Null => Value::Null,
                        JsValue::Bool(b) => Value::Bool(b),
                        JsValue::Number(n) => Value::Number(n),
                        JsValue::String(s) => Value::String(js_str(&s)),
                        _ => Value::Undefined,
                    },
                    pm_eval::node::Got::Undefined => Value::Undefined,
                }),
        )
    }

    fn motion_path(
        &self,
        layer: &JsStr,
        channel: &JsStr,
        progress: f64,
        start: f64,
        end: f64,
    ) -> Option<Result<f64, String>> {
        let cx = self.1?;
        let layer = JsString::from_utf16_units(layer.units());
        let channel = JsString::from_utf16_units(channel.units());
        Some(cx.motion_path(&layer, &channel, progress, start, end))
    }

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
        let converted;
        // Literal keys are the interned strings themselves: match by identity
        // first, then by value (a key computed at run time).
        let interned = self
            .0
            .iter()
            .find(|(s, _)| s.same(key))
            .or_else(|| self.0.iter().find(|(s, _)| s == key));
        let key = if let Some((_, interned)) = interned {
            interned
        } else {
            converted = JsString::from_utf16_units(key.units());
            &converted
        };
        match object {
            Obj::Comp(comp) => match comp.get(key.as_bytes()) {
                pm_eval::node::Got::Node(n) => Some(NodeHost::node(n)),
                _ => None,
            },
            Obj::Node(n) => match n.kind() {
                NodeKind::Object(map) => map.get(key).map(NodeHost::node),
                _ => None,
            },
            Obj::Value(pm_model::Value::Object(map)) => map.get_key(key).map(NodeHost::value),
            Obj::Value(_) => None,
        }
    }
}

impl CompiledExpression for Compiled {
    fn needs_idx(&self) -> bool {
        self.index.is_none() && self.source.expression.needs_idx()
    }

    fn needs_frame(&self) -> bool {
        false
    }

    fn bind(&self, cx: &ExprContext<'_>) -> Option<Arc<dyn CompiledExpression>> {
        if self.source.expression.has_general_binding() {
            let host = NodeHost(&self.source.keys, None);
            let ctx = self.binding_context(cx);
            let (bound, binding) = self.source.expression.bind_general(&host, &ctx)?;
            return Some(Arc::new(Self {
                source: self.source.clone(),
                bound: Some(bound),
                binding_bits: None,
                general: Some(binding),
                index: Some(cx.idx),
                verified: Mutex::new(None),
            }));
        }
        let stamp = self.source.stamp(cx);
        if let Some((previous, plan)) = &*self
            .source
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            && *previous == stamp
            && let Some(plan) = plan.upgrade()
        {
            return Some(plan);
        }
        let bits: Arc<[u64]> = self.binding_bits(cx)?.into();
        let mut pool = self
            .source
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = pool.entries.get(bits.as_ref()).and_then(Weak::upgrade) {
            *self
                .source
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some((stamp, Arc::downgrade(&existing)));
            return Some(existing);
        }
        let host = NodeHost(&self.source.keys, None);
        let ctx = self.binding_context(cx);
        let compiled = Arc::new(Self {
            source: self.source.clone(),
            bound: Some(self.source.expression.bind_numeric(&host, &ctx)?),
            binding_bits: Some(bits.clone()),
            general: None,
            index: Some(cx.idx),
            verified: Mutex::new(None),
        });
        // The cache is only a reuse hint; live slots retain their own plans.
        if !pool.entries.contains_key(bits.as_ref()) {
            if pool.entries.len() == 256 {
                let oldest = pool.order.pop_front().expect("bounded pool order");
                pool.entries.remove(oldest.as_ref());
            }
            pool.order.push_back(bits.clone());
        }
        pool.entries.insert(bits, Arc::downgrade(&compiled));
        *self
            .source
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((stamp, Arc::downgrade(&compiled)));
        Some(compiled)
    }

    fn binding_matches(&self, cx: &ExprContext<'_>) -> Option<bool> {
        if let Some(binding) = &self.general {
            let stamp = self.source.stamp(cx);
            if let Some((previous, result)) = *self
                .verified
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                && previous == stamp
            {
                return Some(result);
            }
            // The bound program keeps the index it was bound at.
            let same_index = !self.source.expression.needs_idx()
                || self.index.map(f64::to_bits) == Some(cx.idx.to_bits());
            let host = NodeHost(&self.source.keys, None);
            let result = same_index
                && self.source.expression.general_binding_matches(
                    &host,
                    &self.binding_context(cx),
                    binding,
                );
            *self
                .verified
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((stamp, result));
            return Some(result);
        }
        let bits = self.binding_bits.as_ref()?;
        if bits.is_empty() {
            return Some(true);
        }
        let stamp = self.source.stamp(cx);
        if let Some((previous, result)) = *self
            .verified
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            && previous == stamp
        {
            return Some(result);
        }
        let host = NodeHost(&self.source.keys, None);
        let ctx = self.binding_context(cx);
        let fixed = if self.source.expression.needs_idx() {
            // The index is the last fixed input (see `binding_bits`).
            let (index, fixed) = bits.split_last()?;
            if *index != cx.idx.to_bits() {
                *self
                    .verified
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((stamp, false));
                return Some(false);
            }
            fixed
        } else {
            bits.as_ref()
        };
        let result = self
            .source
            .expression
            .numeric_fixed_matches(&host, &ctx, fixed);
        *self
            .verified
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((stamp, result));
        Some(result)
    }

    fn binding_is_layer_local(&self) -> bool {
        (self.binding_bits.is_some() || self.general.is_some()) && self.source.layer_local
    }

    fn evaluate_scalars(&self, t: f64, time: f64, value: f64) -> Option<f64> {
        self.expression().evaluate_scalars(t, time, value)
    }

    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
        // The complete evaluator answers `sample` and `motionPath` from the
        // context; bindings and numeric plans never contain them.
        let host = NodeHost(&self.source.keys, Some(cx));
        let ctx = Context {
            t: cx.t,
            time: cx.time,
            fps: cx.fps.to_number(),
            frame: cx.frame.map_or(f64::NAN, |f| f as f64),
            value: NodeHost::js(cx.value),
            layer: cx.layer.map_or(Value::Undefined, NodeHost::node),
            comp: Value::Object(Object::Doc(Obj::Comp(cx.comp))),
            channel: if self.source.channel {
                js_str(cx.channel)
            } else {
                self.source.empty.clone()
            },
            idx: self.index.unwrap_or(cx.idx),
        };
        let out = self.expression().evaluate_untracked(&host, &ctx);
        match out {
            Ok(Value::Number(n)) => Ok(JsValue::Number(n)),
            Ok(Value::Bool(b)) => Ok(JsValue::Bool(b)),
            Ok(Value::String(s)) => Ok(JsValue::String(JsString::from_utf16_units(s.units()))),
            Ok(Value::Null) => Ok(JsValue::Null),
            // Objects and `undefined` are rejected by `evalProp` alike
            // ("did not return a finite value"); the evaluator only checks
            // for a finite number, a string, or a boolean.
            Ok(_) => Ok(JsValue::Undefined),
            Err(e) => Err(ExprError {
                message: Some(e.message()),
            }),
        }
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use pm_model::{Document, Patch};

    #[test]
    fn equal_source_and_numeric_shape_share_a_plan_and_revalidate_exactly() {
        let json = r#"{"id":"P","compId":"C","params":{"amp":{"value":7}},"comps":{"C":{"id":"C","fps":30,"layers":[{"id":"A"},{"id":"B"}]}}}"#;
        let mut doc = Document::new(Some(pm_model::json::parse(json).unwrap()), 1);
        let source = PmExpressions
            .compile(&"param(\"amp\") + comp.layers.length + value".into())
            .unwrap();
        let bound = {
            let cx = Comp::resolve(&doc, "C").unwrap();
            let fps = cx.expression_fps();
            let base = JsValue::Number(1.0);
            let key = JsString::from("opacity");
            let context = ExprContext {
                t: 0.0,
                time: 0.0,
                fps: &fps,
                frame: None,
                value: &base,
                layer: Some(&cx.layers()[0]),
                comp: &cx,
                channel: &key,
                idx: 0.0,
            };
            let first = source.bind(&context).unwrap();
            let other = ExprContext {
                layer: Some(&cx.layers()[1]),
                idx: 1.0,
                ..context
            };
            assert!(Arc::ptr_eq(&first, &source.bind(&other).unwrap()));
            assert_eq!(first.binding_matches(&context), Some(true));
            assert_eq!(first.evaluate_scalars(0.0, 0.0, 4.0), Some(13.0));
            first
        };
        for value in [
            pm_model::Value::Number(-0.0),
            pm_model::Value::String("bad".into()),
        ] {
            doc.apply(&[Patch {
                path: vec!["params".into(), "amp".into(), "value".into()],
                exists: true,
                value: Some(value.clone()),
            }])
            .unwrap();
            let cx = Comp::resolve(&doc, "C").unwrap();
            let fps = cx.expression_fps();
            let base = JsValue::Number(1.0);
            let key = JsString::from("opacity");
            let context = ExprContext {
                t: 0.0,
                time: 0.0,
                fps: &fps,
                frame: None,
                value: &base,
                layer: Some(&cx.layers()[0]),
                comp: &cx,
                channel: &key,
                idx: 0.0,
            };
            assert_eq!(bound.binding_matches(&context), Some(false));
            if matches!(value, pm_model::Value::String(_)) {
                assert!(
                    source.bind(&context).is_none(),
                    "nonnumeric fixed input stays unbound"
                );
                assert_eq!(
                    source.evaluate(&context).unwrap(),
                    JsValue::String("bad21".into())
                );
            } else {
                let next = source.bind(&context).unwrap();
                assert!(!Arc::ptr_eq(&bound, &next));
                assert_eq!(next.binding_matches(&context), Some(true));
                assert_eq!(next.evaluate_scalars(0.0, 0.0, 1.0), Some(3.0));
            }
        }
    }
}
