//! Pure evaluation of a composition at a time (docs/rust-engine.md, section 7).
//!
//! `pm-eval` answers what the legacy evaluator (`legacy/core/anim.ts` and the
//! files it calls) answers, as a pure function of a [`pm_model::Document`], a
//! composition, and a time: no global composition, time, or `PM.scope`.
//!
//! # Modules
//!
//! * [`js_sem`]: JavaScript value semantics evaluation depends on (`+` on
//!   strings, `ToNumber`, loose `==`, `Math.round`, `%`, radix strings).
//! * [`math`]: transcendental functions from `libm` on every target.
//! * [`node`]: JavaScript property access over the document tree.
//! * [`temporal`]: the After Effects speed and influence model
//!   (`core/anim/temporal-ease.ts`).
//! * [`easing`]: the legacy easing library (`legacy/core/easing.ts`): bezier
//!   solver, presets, the float32 spring table.
//! * [`keyframes`]: `evalKfs` and the dense temporal path
//!   (`legacy/core/temporal-bridge.ts`), including hold, linear, bezier,
//!   auto-bezier, continuous, springs, per-byte color interpolation, and
//!   discrete strings and booleans.
//! * [`address`]: property addressing (`findProp`, `allProps`, every key
//!   namespace).
//! * [`comp`]: a composition of a document revision, layer indexes, group
//!   ancestry.
//! * [`expr`]: the interface `pm-expr` implements.
//! * [`evaluator`]: `ev`, `evP`, `active`, `resolveContent`, hooks, and the
//!   interned-id fast path with revision-keyed caches.
//!
//! # Exactness
//!
//! Every floating-point expression keeps the TypeScript operation order (no
//! fused multiply-add, no reassociation). Channel values match the
//! TypeScript implementation bit for bit on the generated vectors
//! (`tests/vectors/`); where the legacy code's answer depends on evaluation
//! history (it mutates and caches live objects), the engine answers as a
//! freshly loaded project does, as each module documents.
#![forbid(unsafe_code)]

pub mod address;
pub mod anchor;
pub mod comp;
pub mod easing;
pub mod evaluator;
pub mod expr;
pub mod groups;
pub mod js_sem;
pub mod keyframes;
pub mod math;
pub mod matte;
pub mod node;
pub mod parenting;
pub mod retime;
pub mod scope;
pub mod space3d;
pub mod temporal;
pub mod transform;

pub use comp::{Comp, CompError};
pub use evaluator::{
    CacheStats, Content, ErrorKey, EvalHooks, Evaluator, NoHooks, PropAddress, PropEval, PropId,
    ValueKind, Via,
};
pub use expr::{CompiledExpression, ExprContext, ExprDeps, ExprError, ExpressionEngine};
pub use js_sem::{JsResult, JsThrow, JsValue};
pub use scope::Scope;
