//! The interface between channel evaluation and the expression language.
//!
//! The parser and interpreter (`legacy/core/expression.ts`) are ported in
//! `pm-expr`, which implements [`ExpressionEngine`]. `pm-eval` owns what
//! surrounds an expression in `evalProp` (`legacy/core/anim.ts`): when it
//! runs, what it sees, which results it accepts, and the per-channel error
//! text. `pm-expr` depends on `pm-eval` (for [`JsValue`], [`crate::js_sem`],
//! and [`crate::math`]), not the other way round, so an evaluator without an
//! engine reports every expression as invalid syntax.

use std::sync::Arc;

use pm_model::{JsString, Node, NodeKind};

use crate::address;
use crate::comp::Comp;
use crate::easing::SpringTables;
use crate::js_sem::{self, JsResult, JsValue};
use crate::keyframes;
use crate::motion_path::PathTiming;
use crate::node::{self, Got};

/// Compiles expression source text.
pub trait ExpressionEngine: Send + Sync {
    /// `compileExpression(source)`: `None` when the source does not compile
    /// (including the engine's own length limits). `pm-eval` only passes
    /// strings; any other `expr` value is invalid syntax without a call.
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>>;
}

/// What an expression's result may depend on beyond its inputs `t`, `T`,
/// `fps`, `frame`, `value`, and `ch`. Evaluation caches use this to decide
/// which edits invalidate a cached result; an engine that cannot tell
/// answers [`ExprDeps::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExprDeps {
    /// Reads members of `layer`.
    pub layer: bool,
    /// Reads members of `comp` or calls `param(...)`.
    pub comp: bool,
    /// Reads `idx` (the layer's index in the composition).
    pub index: bool,
}

impl ExprDeps {
    pub const ALL: ExprDeps = ExprDeps {
        layer: true,
        comp: true,
        index: true,
    };
    pub const NONE: ExprDeps = ExprDeps {
        layer: false,
        comp: false,
        index: false,
    };
}

/// A compiled expression.
pub trait CompiledExpression: Send + Sync {
    /// `needsIdx`: whether `idx` must be computed (`/\bidx\b/` on the source).
    fn needs_idx(&self) -> bool;

    /// Whether the embedding language can read the native `frame` input.
    /// The default preserves existing engines; the legacy dialect has no frame.
    fn needs_frame(&self) -> bool {
        true
    }

    /// See [`ExprDeps`].
    fn dependencies(&self) -> ExprDeps {
        ExprDeps::ALL
    }

    /// Optional revision-local binding of document inputs. The evaluator drops
    /// it on every root revision. Time and the keyed base must remain dynamic.
    fn bind(&self, _cx: &ExprContext<'_>) -> Option<Arc<dyn CompiledExpression>> {
        None
    }

    /// Whether a bound plan's fixed inputs still equal this context exactly.
    /// `None` cannot prove a binding valid; callers rebind via the source.
    fn binding_matches(&self, _cx: &ExprContext<'_>) -> Option<bool> {
        None
    }

    /// All bound inputs are in the layer or its index. Only used when the
    /// caller has proved both unchanged; custom engines default conservative.
    fn binding_is_layer_local(&self) -> bool {
        false
    }

    /// Optional revision-bound numeric plan needing no document context.
    /// The caller uses only finite answers; all other cases keep `evaluate`.
    fn evaluate_scalars(&self, _t: f64, _time: f64, _value: f64) -> Option<f64> {
        None
    }

    /// Runs the expression. An `Err` is a thrown error; its text becomes the
    /// channel's expression error (`e.message`, or `Expression failed.` when
    /// the engine has none).
    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError>;
}

/// A thrown expression error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError {
    /// `Error.message`; `None` for a thrown value that is not an `Error`.
    pub message: Option<String>,
}

/// What an expression sees (`ExpressionContext` in `expression.ts`), plus
/// the integer `frame` of plan section 5, rule 9.
#[derive(Clone, Copy, Debug)]
pub struct ExprContext<'a> {
    /// Layer-local time, `T - layer.from`.
    pub t: f64,
    /// Composition time.
    pub time: f64,
    /// `comp.fps || proj.fps` as stored.
    pub fps: &'a JsValue,
    /// The composition frame containing `time` at the composition's exact
    /// rate, when the rate is valid.
    pub frame: Option<i64>,
    /// The channel's keyframed or static value.
    pub value: &'a JsValue,
    /// The layer (`None` for `undefined`).
    pub layer: Option<&'a Node>,
    /// The composition, read as the legacy object ([`Comp::member`]).
    pub comp: &'a Comp<'a>,
    /// The channel key (`ch`).
    pub channel: &'a JsString,
    /// The layer's index in the composition when [`CompiledExpression::needs_idx`],
    /// otherwise 0.
    pub idx: f64,
}

impl ExprContext<'_> {
    /// `param(name)`: `comp.params && comp.params[name]`, then its `value`,
    /// or 0 when there is no such parameter.
    pub fn param(&self, name: &JsString) -> JsResult<JsValue> {
        let params = self.comp.get(b"params");
        if !params.truthy() {
            return Ok(JsValue::Number(0.0));
        }
        // `params` is a member, so a node; `params[name]` may be computed
        // (a string's character or `length`), whose `value` is undefined.
        let p = match params {
            Got::Node(n) => node::get(n, name.as_bytes()),
            _ => Got::Undefined,
        };
        if !p.truthy() {
            return Ok(JsValue::Number(0.0));
        }
        Ok(match p {
            Got::Node(n) => node::get(n, b"value").to_js(),
            _ => JsValue::Undefined,
        })
    }
}

impl<'a> ExprContext<'a> {
    /// `comp.layers?.find(item => item.id === layerId)`.
    fn find_layer(&self, layer_id: &JsString) -> Result<Option<&'a Node>, String> {
        let layers = match self.comp.get(b"layers") {
            Got::Node(n) => n,
            _ => return Ok(None),
        };
        let NodeKind::Array(items) = layers.kind() else {
            return Err("comp.layers?.find is not a function".into());
        };
        let id = JsValue::String(layer_id.clone());
        for item in items.iter() {
            if node::is_nullish(Some(item)) {
                let base = if matches!(item.kind(), NodeKind::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return Err(format!("Cannot read properties of {base} (reading 'id')"));
            }
            if js_sem::strict_eq(&node::get(item, b"id").to_js(), &id) {
                return Ok(Some(item));
            }
        }
        Ok(None)
    }

    /// The expression helper `sample(layerId, channel, localTime)` (`anim.ts`):
    /// the layer's raw channel (`PM.findProp`) at a local time, its keys
    /// evaluated without expressions (`evalKfs`), or its static value. `Err`
    /// is the thrown `Error`'s message.
    pub fn sample(
        &self,
        layer_id: &JsString,
        channel: &JsString,
        local_time: f64,
    ) -> Result<Got<'a>, String> {
        if !local_time.is_finite() {
            return Err("Sample time must be finite".into());
        }
        let source = match self.find_layer(layer_id)? {
            Some(layer) => address::find_prop(layer, channel).map_err(|e| e.message)?,
            None => Got::Undefined,
        };
        if !source.truthy() {
            return Err("Sampled channel is unavailable".into());
        }
        let Some(prop) = source.node() else {
            // A computed value (a string's character): `kf` and `v` are absent.
            return Ok(Got::Undefined);
        };
        let kf = node::get(prop, b"kf");
        let keyed = match kf.node() {
            Some(k) => node::get(k, b"length").truthy(),
            None => false,
        };
        if keyed {
            let mut springs = SpringTables::new();
            return keyframes::evaluate(kf.node(), local_time, false, &mut springs)
                .map(Got::Value)
                .map_err(|e| e.message);
        }
        Ok(node::get(prop, b"v"))
    }

    /// The expression helper `motionPath(layerId, channel, progress, start,
    /// end)` (`anim.ts` `motionPathValue`): the position axis `channel` at the
    /// arc-length fraction `progress` of the layer's original position curves
    /// between the local times `start` and `end` ([`PathTiming`]).
    pub fn motion_path(
        &self,
        layer_id: &JsString,
        channel: &JsString,
        progress: f64,
        start: f64,
        end: f64,
    ) -> Result<f64, String> {
        const AXES: [&[u8]; 3] = [b"position.x", b"position.y", b"position.z"];
        if !AXES.contains(&channel.as_bytes())
            || ![progress, start, end].iter().all(|v| v.is_finite())
            // `!(end > start)`; both are finite here.
            || end <= start
        {
            return Err("Invalid motion path request".into());
        }
        let layer = self
            .find_layer(layer_id)?
            .ok_or_else(|| "Motion path layer is unavailable".to_owned())?;
        // `layer.p?.[axis]`, kept when truthy.
        let p = node::member(Some(layer), "p");
        let axes: Vec<(&[u8], &Node)> = AXES
            .iter()
            .filter_map(|axis| match p.map(|p| node::get(p, axis)) {
                Some(Got::Node(n)) if node::truthy(Some(n)) => Some((*axis, n)),
                _ => None,
            })
            .collect();
        let index = axes
            .iter()
            .position(|(axis, _)| *axis == channel.as_bytes())
            .ok_or_else(|| "Motion path axis is unavailable".to_owned())?;
        // `channels.flatMap(prop => prop.kf.map(k => k.t))`.
        let mut knots = Vec::new();
        for (_, prop) in &axes {
            let kf = node::get(prop, b"kf");
            let items = match kf.node().map(Node::kind) {
                Some(NodeKind::Array(items)) => items,
                _ if kf.is_nullish() => {
                    return Err("Cannot read properties of undefined (reading 'map')".into());
                }
                _ => return Err("prop.kf.map is not a function".into()),
            };
            for k in items.iter() {
                if node::is_nullish(Some(k)) {
                    return Err("Cannot read properties of null (reading 't')".into());
                }
                if let NodeKind::Number(t) = node::get(k, b"t")
                    .node()
                    .map_or(&NodeKind::Null, Node::kind)
                {
                    knots.push(*t);
                }
            }
        }
        // Each track is prepared once: the timing table samples it often.
        let mut springs = SpringTables::new();
        let tracks: Vec<(Option<&Node>, Option<keyframes::PreparedTrack>, JsValue)> = axes
            .iter()
            .map(|(_, prop)| {
                let kf = node::get(prop, b"kf").node();
                let keyed = kf.is_some_and(|k| node::get(k, b"length").truthy());
                let track = keyed.then(|| keyframes::PreparedTrack::prepare(kf));
                (kf, track, node::get(prop, b"v").to_js())
            })
            .collect();
        let mut sample = |time: f64| -> Result<Vec<f64>, String> {
            let mut out = Vec::with_capacity(tracks.len());
            for (kf, track, v) in &tracks {
                let value = match track {
                    Some(track) => track
                        .value(*kf, time, false, &mut springs)
                        .map_err(|e| e.message)?,
                    None => v.clone(),
                };
                out.push(value.to_number());
            }
            Ok(out)
        };
        let timing = PathTiming::new(&mut sample, &knots, start, end)?;
        Ok(timing.position(&mut sample, progress)?[index])
    }
}

/// The expression error texts of `evalProp`.
pub mod messages {
    pub const NOT_FINITE: &str = "Expression did not return a finite value.";
    pub const FAILED: &str = "Expression failed.";
    pub const INVALID: &str = "Expression syntax is invalid.";
}
