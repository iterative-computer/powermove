//! The project expression language (docs/rust-engine.md, sections 3, 5, and 7).
//!
//! A port of `legacy/core/expression.ts`, the bounded parser and interpreter
//! that evaluates channel expressions without running JavaScript. Everything a
//! project can observe is reproduced: the grammar and its limits (2,000 UTF-16
//! code units, 160 nodes, depth 24, 12 call arguments), every error message,
//! `expressionDiagnostic` and `EXPRESSION_NAMES`, the identifiers and helpers,
//! member and index access into raw project values, loose equality and string
//! coercion, and `Math` results bit for bit with the V8 in Powermove's Electron
//! (see [`jsmath`]). The quirks of the original that projects can reach are
//! kept too (see [`parser`](mod@parser)'s notes and `docs/engine/expressions.md`).
//!
//! # Use
//!
//! ```
//! use pm_expr::{Context, Dialect, Expression, JsonHost, Value};
//!
//! let layer = pm_model::json::parse(r#"{"name":"Title","p":{"x":{"v":4}}}"#).unwrap();
//! let comp = pm_model::json::parse(r#"{"layers":[],"params":{"gain":{"value":3}}}"#).unwrap();
//! let expr = Expression::compile("layer.p.x.v * param('gain') + t", Dialect::Native).unwrap();
//! let ctx = Context {
//!     t: 0.5,
//!     time: 2.0,
//!     fps: 30.0,
//!     frame: 60.0,
//!     value: Value::Number(10.0),
//!     layer: JsonHost::value(&layer),
//!     comp: JsonHost::value(&comp),
//!     channel: "position.x".into(),
//!     idx: 0.0,
//! };
//! let out = expr.evaluate(&JsonHost::new(), &ctx);
//! assert_eq!(out.result.unwrap().as_number(), Some(12.5));
//! assert!(out.deps.depends_on_time());
//! ```
//!
//! # Dependencies
//!
//! Every evaluation returns the context inputs and document fields it read
//! ([`Dependencies`]). Reads are recorded as they happen, so they describe the
//! branch taken: if none of them changes, the result cannot change.

// The only unsafe code is the optional wasm import of `Math.pow` (feature
// `js-pow`), a declaration of a pure function.
#![deny(unsafe_code)]

mod deps;
mod diagnostic;
mod eval;
mod host;
pub mod jsmath;
mod lexer;
mod parser;
mod string;
mod value;

pub use deps::{ContextReads, Dependencies, FieldRead, Root, Step};
pub use diagnostic::{
    LENGTH_MESSAGE, expression_diagnostic, expression_diagnostic_utf16, expression_names,
};
pub use eval::EvalError;
pub use host::{Host, JsonHost, JsonRef, Shape};
pub use lexer::SyntaxError;
pub use parser::{MAX_CALL_ARGS, MAX_DEPTH, MAX_NODES, MAX_SOURCE_CHARS};
pub use string::JsStr;
pub use value::{
    Callable, Helper, MATH_CONSTANTS, MathFunction, Object, Value, number_to_string,
    string_to_number,
};

use eval::Evaluator;
use parser::Node;

/// Which names exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// Exactly `expression.ts`: `frame` is an unknown name (`undefined`).
    Legacy,
    /// The engine's language: `Legacy` plus the integer `frame` (section 5,
    /// rule 9). Expressions that do not mention `frame` behave identically.
    #[default]
    Native,
}

/// The inputs of one evaluation (`ExpressionContext` in `expression.ts`).
#[derive(Clone, Debug)]
pub struct Context<O> {
    /// `t`: layer-local time in seconds.
    pub t: f64,
    /// `time` and `T`: composition time in seconds.
    pub time: f64,
    /// `fps`: the composition's frame rate as stored.
    pub fps: f64,
    /// `frame` (native dialect): the composition frame index at `time`,
    /// computed by the caller with `pm_time` frame math (an integer).
    pub frame: f64,
    /// `value`: the channel's keyframed (or static) value.
    pub value: Value<O>,
    /// `layer`: the raw layer object.
    pub layer: Value<O>,
    /// `comp`: the raw composition object; `param(name)` reads
    /// `comp.params[name].value`.
    pub comp: Value<O>,
    /// `ch`: the channel key.
    pub channel: JsStr,
    /// `idx`: the layer's index in the composition. `expression.ts` computes it
    /// only when [`Expression::needs_idx`] and passes 0 otherwise.
    pub idx: f64,
}

/// A parsed expression (`compileExpression`). Immutable, `Send + Sync`, and
/// cheap to evaluate many times.
#[derive(Clone, Debug)]
pub struct Expression {
    tree: Node,
    needs_idx: bool,
    dialect: Dialect,
}

/// Why a source does not compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Empty, or longer than [`MAX_SOURCE_CHARS`] UTF-16 code units.
    Length,
    Syntax(SyntaxError),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Length => f.write_str("Expression is empty or too long"),
            CompileError::Syntax(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CompileError {}

/// The result of an evaluation and what it read.
#[derive(Clone, Debug)]
pub struct Evaluation<O> {
    pub result: Result<Value<O>, EvalError>,
    pub deps: Dependencies,
}

/// A channel's expression result after the checks `evalProp` applies.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelValue {
    Number(f64),
    String(JsStr),
    Bool(bool),
}

/// What `evalProp` does with an expression: replace the keyframed value, or
/// keep it and record an error for `expressionErrors`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelEvaluation {
    /// `Some` when the expression's result replaces the keyframed value.
    pub value: Option<ChannelValue>,
    /// The `expressionErrors` entry, `None` when the result was used.
    pub error: Option<String>,
    pub deps: Dependencies,
}

/// `PM.expressionErrors` message when the source does not compile.
pub const SYNTAX_INVALID_MESSAGE: &str = "Expression syntax is invalid.";
/// `PM.expressionErrors` message when the result is not a finite number,
/// string, or boolean.
pub const NOT_FINITE_MESSAGE: &str = "Expression did not return a finite value.";

fn needs_idx(source: &[u16]) -> bool {
    // `/\bidx\b/`: word characters are [A-Za-z0-9_].
    let word = |u: u16| matches!(u, 0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a | 0x5f);
    let pattern = [u16::from(b'i'), u16::from(b'd'), u16::from(b'x')];
    source.windows(3).enumerate().any(|(i, w)| {
        w == pattern
            && (i == 0 || !word(source[i - 1]))
            && source.get(i + 3).is_none_or(|&u| !word(u))
    })
}

impl Expression {
    /// `compileExpression(source)`: `None` for an empty or too long source or
    /// a syntax error.
    pub fn compile(source: &str, dialect: Dialect) -> Option<Expression> {
        Expression::parse(source, dialect).ok()
    }

    /// Like [`compile`](Expression::compile), with the reason for a failure.
    pub fn parse(source: &str, dialect: Dialect) -> Result<Expression, CompileError> {
        let units: Vec<u16> = source.encode_utf16().collect();
        Expression::parse_utf16(&units, dialect)
    }

    /// [`parse`](Expression::parse) for a JavaScript string given as UTF-16
    /// code units (project JSON may hold lone surrogates).
    pub fn parse_utf16(units: &[u16], dialect: Dialect) -> Result<Expression, CompileError> {
        if units.is_empty() || units.len() > MAX_SOURCE_CHARS {
            return Err(CompileError::Length);
        }
        let tree = parser::parse(units).map_err(CompileError::Syntax)?;
        Ok(Expression {
            tree,
            needs_idx: needs_idx(units),
            dialect,
        })
    }

    /// [`compile`](Expression::compile) for UTF-16 code units.
    pub fn compile_utf16(units: &[u16], dialect: Dialect) -> Option<Expression> {
        Expression::parse_utf16(units, dialect).ok()
    }

    /// Whether the source mentions `idx` as a word (the caller computes the
    /// layer index only then, as `anim.ts` does).
    pub fn needs_idx(&self) -> bool {
        self.needs_idx
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Evaluates against raw project values supplied by `host`.
    pub fn evaluate<H: Host>(&self, host: &H, ctx: &Context<H::Obj>) -> Evaluation<H::Obj> {
        let mut evaluator = Evaluator::new(host, ctx, self.dialect);
        let result = evaluator
            .evaluate(&self.tree)
            .map(|v| v.map_doc(|t| t.into_obj()));
        Evaluation {
            result,
            deps: evaluator.dependencies(),
        }
    }
}

/// `String(value)` as V8 computes it, reading document objects through
/// `host` (for showing a result, or comparing one with JavaScript's).
pub fn to_js_string<H: Host>(host: &H, value: &Value<H::Obj>) -> Result<JsStr, EvalError> {
    let ctx = Context {
        t: 0.0,
        time: 0.0,
        fps: 0.0,
        frame: 0.0,
        value: Value::Undefined,
        layer: Value::Undefined,
        comp: Value::Undefined,
        channel: JsStr::default(),
        idx: 0.0,
    };
    Evaluator::new(host, &ctx, Dialect::Native).string_of(value)
}

/// `evalProp`'s handling of an expression: `compiled` is `None` when the
/// source did not compile.
pub fn evaluate_channel<H: Host>(
    compiled: Option<&Expression>,
    host: &H,
    ctx: &Context<H::Obj>,
) -> ChannelEvaluation {
    let Some(expr) = compiled else {
        return ChannelEvaluation {
            value: None,
            error: Some(SYNTAX_INVALID_MESSAGE.into()),
            deps: Dependencies::default(),
        };
    };
    let Evaluation { result, deps } = expr.evaluate(host, ctx);
    let (value, error) = match result {
        Ok(Value::Number(n)) if n.is_finite() => (Some(ChannelValue::Number(n)), None),
        Ok(Value::String(s)) => (Some(ChannelValue::String(s)), None),
        Ok(Value::Bool(b)) => (Some(ChannelValue::Bool(b)), None),
        Ok(_) => (None, Some(NOT_FINITE_MESSAGE.into())),
        Err(e) => (None, Some(e.message())),
    };
    ChannelEvaluation { value, error, deps }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_idx_is_a_word_match() {
        let n = |s: &str| needs_idx(&s.encode_utf16().collect::<Vec<_>>());
        assert!(n("idx"));
        assert!(n("idx*2"));
        assert!(n("$idx"));
        assert!(!n("idx2"));
        assert!(!n("_idx"));
        assert!(!n("Idx"));
    }

    #[test]
    fn expression_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Expression>();
    }
}
