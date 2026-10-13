//! The evaluation scope layer queries run in: an [`Evaluator`], the
//! composition being evaluated (`PM.curComp()`), and the open composition
//! (`PM.proj`).
//!
//! The legacy layer queries (`localMatrix`, `worldMatrix`, the 2.5D space,
//! bounds, picking, text animation, ...) read three globals: the evaluator
//! state, `PM.curComp()` (the top of `PM.scope`, the composition a nested
//! render is drawing), and `PM.proj` (the open composition, whose size some
//! fallbacks read even inside a nested composition). [`Scope`] carries all
//! three explicitly, so every query is a pure function of its arguments.
//!
//! Lookups follow the legacy indexes exactly:
//!
//! - [`Scope::parent_of`] is `anim.ts`'s `parentOf`: the *first* layer of
//!   `comp.layers` with the parent's id (any id type, `SameValueZero`).
//! - [`Scope::layer_by_id`] is `PM.L`: the *last* layer with a string id
//!   (`ProjectIndex.byId`); non-string ids find nothing.
//! - [`Scope::group_ancestors`] is `PM.groupAncestors` (last layer per id).

use std::sync::Arc;

use pm_model::{JsString, Node};

use crate::comp::{Comp, IdKey, LayerIndex};
use crate::evaluator::Evaluator;
use crate::js_sem::{JsResult, JsThrow, JsValue};
use crate::node::{self, Got};

/// A composition being evaluated, with the open composition beside it.
pub struct Scope<'a, 'd> {
    pub ev: &'a mut Evaluator,
    /// `PM.curComp()`.
    pub comp: &'a Comp<'d>,
    /// `PM.proj`: the open composition (the same as `comp` unless a nested
    /// composition is being evaluated).
    pub project: &'a Comp<'d>,
}

impl<'a, 'd> Scope<'a, 'd> {
    pub fn new(ev: &'a mut Evaluator, comp: &'a Comp<'d>, project: &'a Comp<'d>) -> Self {
        Scope { ev, comp, project }
    }

    /// The open composition evaluated as itself (`PM.scope` empty).
    pub fn open(ev: &'a mut Evaluator, comp: &'a Comp<'d>) -> Self {
        Scope {
            ev,
            comp,
            project: comp,
        }
    }

    /// The composition's layers (`comp.layers`, or none).
    pub fn layers(&self) -> &'d [Node] {
        self.comp.layers()
    }

    pub(crate) fn index(&mut self) -> Arc<LayerIndex> {
        self.ev.layer_index(self.comp)
    }

    /// `parentOf(L)` (`anim.ts`): the first layer whose `id` is `L.parent`.
    /// Throws when some element of `comp.layers` is `null` (building the
    /// index reads every layer's `id`).
    pub fn parent_of(&mut self, layer: &'d Node) -> JsResult<Option<&'d Node>> {
        let parent = node::get(layer, b"parent");
        if !parent.truthy() {
            return Ok(None);
        }
        let index = self.index();
        if index.has_nullish {
            return Err(JsThrow::type_error(
                "Cannot read properties of null (reading 'id')",
            ));
        }
        Ok(IdKey::of(&parent)
            .and_then(|k| index.first.get(&k))
            .map(|&i| &self.layers()[i]))
    }

    /// `PM.L(id)`: the last layer with this string id in the composition.
    pub fn layer_by_id(&mut self, id: &Got<'_>) -> Option<&'d Node> {
        let JsValue::String(s) = id.to_js() else {
            return None;
        };
        let index = self.index();
        index
            .last
            .get(&IdKey::String(s))
            .map(|&i| &self.layers()[i])
    }

    /// `PM.L(L.parent)` when `L.parent` is truthy (the 3D hierarchy's lookup).
    pub fn parent_by_id(&mut self, layer: &'d Node) -> Option<&'d Node> {
        let parent = node::get(layer, b"parent");
        if !parent.truthy() {
            return None;
        }
        self.layer_by_id(&parent)
    }

    /// `PM.groupAncestors(L)`: enclosing groups, innermost first.
    pub fn group_ancestors(&mut self, layer: &'d Node) -> JsResult<Vec<&'d Node>> {
        self.ev.group_ancestors(self.comp, layer)
    }

    /// `PM.ev(L, key, T)`.
    pub fn channel(&mut self, layer: &'d Node, key: &str, time: f64) -> JsResult<JsValue> {
        self.ev
            .ev(self.comp, Some(layer), &JsString::from(key), time)
    }

    /// `Number(PM.ev(L, key, T))` (JavaScript's `ToNumber`, as arithmetic
    /// on the result applies it).
    pub fn num(&mut self, layer: &'d Node, key: &str, time: f64) -> JsResult<f64> {
        Ok(self.channel(layer, key, time)?.to_number())
    }

    /// `PM.evP(L, prop, T, key)`.
    pub fn channel_p(
        &mut self,
        layer: &'d Node,
        prop: Option<&'d Node>,
        time: f64,
        key: &str,
    ) -> JsResult<JsValue> {
        self.ev
            .ev_p(self.comp, Some(layer), prop, time, &JsString::from(key))
    }

    /// `isProperty(v) ? PM.evP(L, v, T, key) : v` (`evaluatedValue`).
    pub fn evaluated(
        &mut self,
        layer: &'d Node,
        value: Option<&'d Node>,
        time: f64,
        key: &str,
    ) -> JsResult<JsValue> {
        if node::is_property(value) {
            self.channel_p(layer, value, time, key)
        } else {
            Ok(node::js_value_opt(value))
        }
    }

    /// `PM.active(L, T)`.
    pub fn active(&mut self, layer: &'d Node, time: f64) -> JsResult<bool> {
        self.ev.active(self.comp, Some(layer), time)
    }

    /// `Number(PM.proj.w)` and `Number(PM.proj.h)`.
    pub fn project_size(&self) -> (f64, f64) {
        (
            node::to_number(self.project.member("w")),
            node::to_number(self.project.member("h")),
        )
    }

    /// `Number(comp.w)` and `Number(comp.h)` of the current composition.
    pub fn comp_size(&self) -> (f64, f64) {
        (
            node::to_number(self.comp.member("w")),
            node::to_number(self.comp.member("h")),
        )
    }
}

/// Whether two optional nodes are the same object.
pub(crate) fn same(a: &Node, b: &Node) -> bool {
    std::ptr::eq(a, b)
}

/// `layer.d.paths?.length` truthy (throws when `layer.d` is nullish).
pub(crate) fn has_paths(layer: &Node) -> JsResult<bool> {
    let d = node::get_checked(Some(layer), b"d")?;
    let paths = match &d {
        Got::Node(d) if !node::is_nullish(Some(d)) => node::get(d, b"paths"),
        Got::Value(v) if !matches!(v, JsValue::Undefined | JsValue::Null) => Got::Undefined,
        _ => {
            return Err(JsThrow::type_error(
                "Cannot read properties of undefined (reading 'paths')",
            ));
        }
    };
    Ok(match paths {
        Got::Node(p) if !node::is_nullish(Some(p)) => node::get(p, b"length").truthy(),
        Got::Value(v) => v.to_js_string().utf16_len() > 0,
        _ => false,
    })
}

/// `layer.type === name`.
pub(crate) fn is_type(layer: &Node, name: &str) -> bool {
    node::is_str(node::member(Some(layer), "type"), name)
}

/// `!!layer[key]`.
pub(crate) fn flag(layer: &Node, key: &str) -> bool {
    node::truthy(node::member(Some(layer), key))
}

/// `PM.clamp(v, a, b)`: `v < a ? a : v > b ? b : v` (NaN passes through).
#[inline]
pub(crate) fn clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}

/// `Number(x) || 0` for a number: NaN and `-0` become `+0`.
#[inline]
pub(crate) fn or0(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 { 0.0 } else { x }
}

/// `Math.max(a, b)` (NaN wins; `+0` beats `-0`).
#[inline]
pub(crate) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { b } else { a }
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)` (NaN wins; `-0` beats `+0`).
#[inline]
pub(crate) fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// `Math.hypot(...values)` with V8's algorithm (scaled, Kahan-compensated).
pub(crate) fn js_hypot(values: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for &v in values {
        let a = v.abs();
        if a.is_nan() {
            nan = true;
        } else if a > max {
            max = a;
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut compensation = 0.0f64;
    for &v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}
