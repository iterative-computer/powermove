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

use pm_model::{JsString, Node};

use crate::comp::Comp;
use crate::js_sem::{JsResult, JsValue};
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

/// The expression error texts of `evalProp`.
pub mod messages {
    pub const NOT_FINITE: &str = "Expression did not return a finite value.";
    pub const FAILED: &str = "Expression failed.";
    pub const INVALID: &str = "Expression syntax is invalid.";
}
