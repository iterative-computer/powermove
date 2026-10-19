# Expressions (`pm-expr`)

Design notes for `engine/crates/pm-expr`, the native port of
`apps/desktop/src/renderer/src/legacy/core/expression.ts` (plan sections 3, 5
rule 9, and 7). Everything here was measured on 2026-10-12 against Electron 44
(Chromium 152, V8 15.2.124.13, Apple silicon), the runtime Powermove ships.

## What the crate is

A bounded parser and compiled bytecode evaluator for channel expressions, with no
JavaScript engine. It reproduces what a project can observe of
`expression.ts`:

- the grammar and limits: 2,000 UTF-16 code units, 160 nodes, depth 24, 12 call
  arguments, with the same error messages and positions (UTF-16 indexes);
- `expressionDiagnostic` (`expression_diagnostic`) and `EXPRESSION_NAMES`
  (`expression_names`), checking names in the same visiting order, so the first
  problem reported is the same;
- identifiers `time`, `T`, `t`, `fps`, `value`, `layer`, `comp`, `ch`, `idx`,
  `PI`, `Math`, and `Math.*` constants and functions; helpers `clamp`, `lerp`,
  `linear`, `ease`, `random`, `wiggle`, `bounce`, `loop`, `pingpong`, `param`;
- member and index access into raw project values (own properties only, array
  indexes through `Number.isSafeInteger(Number(key))`, `length`, forbidden
  `__proto__`/`prototype`/`constructor`);
- JavaScript conversions: `ToPrimitive` on project objects and arrays (arrays
  join, objects with an own `toString` property throw), `ToNumber`, `ToString`
  (`Number::toString` via `ryu-js`), loose and strict equality with object
  identity, truthiness, `%` as `fmod`, `Math.round` ties toward +∞;
- the results `evalProp` accepts (finite number, string, boolean; anything else
  keeps the keyframed value) and its three `expressionErrors` messages
  (`evaluate_channel`).

Public API, in short:

| Item | Role |
|---|---|
| `Expression::compile(source, dialect)` / `parse` / `*_utf16` | `compileExpression`; the UTF-16 forms accept sources with lone surrogates (project JSON can hold them) |
| `Expression::evaluate(host, ctx) -> Evaluation` | result (`Value` or `EvalError` with V8's message) plus `Dependencies` |
| `evaluate_channel(compiled, host, ctx)` | `evalProp`: replacement value or the `expressionErrors` message |
| `Host` | read access to the document: `shape`, `element`, `property`; handle equality is object identity |
| `JsonHost` | a `Host` over `pm_model::Value` trees (identity by address) |
| `Context` | `t`, `time`, `fps`, `frame`, `value`, `layer`, `comp`, `channel`, `idx` |
| `Dialect::{Legacy, Native}` | `Native` (default) adds `frame`; `Legacy` is exactly `expression.ts` |
| `expression_diagnostic`, `expression_names`, `to_js_string`, `jsmath` | editor helpers and V8-identical `Math` |

## Quirks kept on purpose

`expression.ts` is plain JavaScript, and some of its accidents are reachable
from project data. They are part of the language now, so the port keeps them
(each is in the parity fixture):

- Tokens compare by value regardless of kind. A string literal `"+"` in
  operator position is `+`; `"("` after a value opens a call; `"-" 3` is `-3`.
- The operator table is a plain object. A token naming an `Object.prototype`
  member (`constructor`, `toString`, `valueOf`, `__proto__`, …) finds a
  function there, so it parses as an inert binary operator (both operands are
  evaluated, the result is `undefined`), and the minimum precedence of its right
  operand becomes a string such as `function toString() { [native code] }1`,
  which later precedences compare with as strings. `1 toString 2 ? 3 : 4` is 4.
  `expressionDiagnostic` accepts these.
- Calls look functions up with `MATH_FUNCTIONS[name]`, which also finds
  `Object.prototype` members: `constructor(5) + 1` is 6 (a Number wrapper),
  `toString()` is `"[object Undefined]"`, `isPrototypeOf(1)` is false, and
  `valueOf()`, `hasOwnProperty(1)`, `toLocaleString()`, and `__proto__(1)`
  throw V8's messages. The diagnostic rejects all of them, but project files
  can contain them.
- Bare helper and function names evaluate to fresh `{ callable: name }`
  objects: `clamp.callable` is `"clamp"`, and `clamp == clamp` is false.
- `param(name)` reads `comp.params[name]` with an ordinary JavaScript lookup,
  so inherited names (`param("constructor")`) give `undefined`, not 0.
- The one place `__proto__(…)` throws, V8's message names the local variable
  of the unminified source (`math is not a function`).

## Math: what V8 15.2 actually computes

The plan (section 7) says `sin`-based `random` and `wiggle` match V8 by using
the `libm` crate. They do not: V8 15.2 implements `Math.sin`, `cos`, `tan`,
`asin`, `acos`, `atan`, `atan2`, `exp`, and `log` with LLVM libc
(`src/base/ieee754.cc` includes `third_party/llvm-libc/src/shared/math.h`), and
`libm` is fdlibm. On 20,000 inputs per function, `libm` differed from Electron
in 606 `sin`, 544 `cos`, 644 `tan`, 1,180 `asin`, 1,561 `acos`, 235 `atan`,
4,290 `atan2`, 2,043 `exp`, 428 `log`, and 1,969 `pow` results. (Node 24's
V8 differs from Electron 44's as well, so Node is not a valid reference.)

Measured against 300-bit references (mpmath, 5,000 inputs per function), Electron's `sin`, `cos`, `tan`,
`asin`, `acos`, `atan2`, `exp`, and `log` are correctly rounded, so any
correctly rounded implementation reproduces them. `atan` and `pow` are not
(`atan` returns `fma(-2^-54, x, x)` below 2^-26; `pow` is off by one unit in
the last place on about 0.09% of inputs). What the crate does:

| Function | Implementation | Why |
|---|---|---|
| `sin`, `cos`, `tan`, `asin`, `acos`, `atan2`, `log` | `pxfm` 0.1.30 (correctly rounded) | V8 is correctly rounded |
| `exp` | a copy of pxfm's CORE-MATH port, every multiply-add fused | pxfm's accurate path returns wrong subnormals (exp(-709.6296676644124) gave 3.68e-309, V8 6.48e-309) |
| `atan` | LLVM libc's `atan` ported line for line (revision 3ea89f43, the one Chromium 152 pins), FMA configuration | not correctly rounded; only the same algorithm matches |
| `pow`, `**` | V8's ECMAScript special cases, then the platform `pow` natively | V8's `--use-std-math-pow` defaults to true (all platforms but AIX), so V8 calls `std::pow`, which on macOS is libsystem_m's |
| `pow` on wasm32 | feature `js-pow`: import the embedding JavaScript's `Math.pow` (`pm_expr.pow`); otherwise a correctly rounded fallback | there is no platform `pow` in wasm; in the editor's renderer the import is V8's own |
| `floor`, `ceil`, `abs`, `sqrt`, `sign`, `round`, `max`, `min` | exact operations with ECMAScript edge cases | |

The LLVM `atan` port uses fused multiply-adds everywhere (as Chromium's arm64
build does). V8 on x86-64 compiles LLVM libc without FMA, so Intel builds of
Electron can differ from it (and from Apple silicon) in rare last bits; the
engine matches Apple silicon on every target.

The fallback `pow` (wasm32 without `js-pow`) handles every C99 Annex F special
case itself; pxfm 0.1.30's `f_pow` gets `pow(NaN, 0)`, negative zero and
negative infinite bases with non-integer exponents wrong, and rounds some
subnormal results wrongly (`pow(2, -1075)` is 5e-324, not 0). It agrees with
macOS's `pow` on 399,652 of 400,000 sampled inputs. `pm-wasm` should enable
`js-pow` and provide `{ pm_expr: { pow: Math.pow } }` when instantiating.

## Parser recursion

`expression.ts` recurses once per prefix token, so 2,000 nested parentheses
recurse about 4,000 frames deep before failing with `Expected a value at
2000`. V8's wasm stack overflowed on exactly that input (`RangeError: Maximum
call stack size exceeded`). Below the depth limit no parse can succeed (every
node created deeper than 24 fails, and none is created before the innermost
operand), so the port finishes with a loop that reaches the same error: the
first lexer error while consuming unary operators and `(`, then a lexer error
on the token after the innermost operand, "Expression is too complex", or
"Expected a value". Parsing now needs a bounded stack (tested in a 256 KiB
thread); evaluation and diagnostics walk trees at most about 25 levels deep.

## `frame`

Plan section 5 rule 9 adds an integer `frame`. In `Dialect::Native` (the
default) `frame` is a known name whose value is `Context::frame`; the caller
computes it with `pm_time` frame math (the composition frame index at `time`,
`frame_of(ticks)` at the composition rate), so the crate never rounds floats
into frames. In `Dialect::Legacy` it is an unknown name evaluating to
`undefined`, exactly as `expression.ts`. Expressions that do not mention
`frame` behave identically in both (checked on every parity case). Until the
editor's diagnostic comes from `pm-wasm`, `expression.ts` still rejects
`frame`, so users cannot type it yet.

## Dependencies

Each evaluation returns `Dependencies`: the context inputs it read
(`ContextReads`: `t`, `time`/`T`, `fps`, `frame`, `value`, `ch`, `idx`; `wiggle`
reads `t`) and every document field it read (`FieldRead`: a root — layer,
comp, or the keyframed value — and a path of `Key`, `Index`, and `Length`
steps). Reads are recorded as they happen, so they describe the branch taken.

A read of a primitive depends on its value; a read of an object or array
depends only on which object is there, because every read inside it is a
longer path of its own. Absent fields are recorded too, so creating one
invalidates. Conversions record what they look at: joining an array reads its
length and every element, converting an object probes its own `toString`.
`param(name)` reads `comp.params`, `comp.params[name]`, and
`comp.params[name].value`. `Dependencies::affected_by(root, path)` answers
whether a change at a path (the node replaced or modified) can change the
result.

For `pm-eval`: when `layer` is an element of `comp.layers`, the same node is
reachable under both roots; map a document write to both (`comp.layers[i].x`
and `layer.x`) before asking `affected_by`. Identity must be stable across a
write that does not replace a node (pm-model's reconciliation keeps nodes), and
`idx` is only computed when `Expression::needs_idx()` is true (as `anim.ts`
does; otherwise it is 0).

The parity suite checks soundness directly: for a third of the fixture cases
it changes, one at a time, context inputs the evaluation did not read and up to
four document fields under each root that no recorded read covers, and asserts
the result does not change (37,313 perturbations, none changed a result).

## Parity harness

`engine/crates/pm-expr/parity/run.ts` (bun; source `engine/env.sh` first):

1. bundles `expression.ts` with `Bun.build`;
2. generates cases: curated edge cases (limits, errors, names, calls, quirks,
   members, coercion) on seven contexts, a coercion matrix (30 atoms × 30 atoms
   × 13 operators), expression-like string literals harvested from the
   repository, seeded grammar fuzz cases, and every expression in the private
   corpus with its real layer and composition at five times (never written into
   the repository);
3. evaluates them with `expression.ts` in V8 (`ELECTRON_RUN_AS_NODE=1` Electron
   44) and, for information, in bun's JavaScriptCore;
4. sweeps 13 functions over 400,000 inputs each (`sweep.mjs`, mirrored by
   `tests/support/mod.rs`): the `Math` functions plus `random(value)` and
   `wiggle(value, 20, 7)` evaluated by `expression.ts`;
5. runs `cargo test -p pm-expr --release --test parity` against the results;
6. builds `examples/wasm_parity.rs` for wasm32 and runs the same cases and sweep
   through it (`wasm.mjs`): with `js-pow` under V8, and without under bun.

`--write-fixture` rewrites `tests/fixtures/parity-{cases,expected}.jsonl`
(public cases only), which `cargo test` checks on every run without Electron.

V8 is the reference, not bun: JavaScriptCore disagrees with V8 on 228 of the
10,529 fixture cases (124 error messages such as `No default value` for
`Cannot convert object to primitive value`, 99 function source texts and
object conversions, and 5 `Math` results such as `wiggle(2, 120)` in the last
bit).

## Measurements (2026-10-12)

| Check | Result |
|---|---|
| Fixture (committed), native | 10,529 of 10,529 cases identical to V8 |
| Full run, native | 42,429 of 42,429 (3,798 edge, 4,500 + 11,700 matrix, 231 repository, 22,000 fuzz, 200 corpus) |
| Full run, wasm32 with `js-pow`, in V8 | 42,429 of 42,429 |
| Full run, wasm32 fallback, in bun | 42,422 of 42,429; the 7 others differ only through `pow` |
| Math sweep, native | 13 functions × 400,000 inputs, all bit-identical |
| Math sweep, wasm32 with `js-pow` | all bit-identical |
| Math sweep, wasm32 fallback | all bit-identical except `pow`: 399,652 of 400,000 |
| Dependency soundness | 37,313 perturbations of unread inputs, 0 changed results |
| Parsing 2,000-deep inputs | bounded stack (256 KiB thread) |
| Evaluation (release, JSON host, `examples/expr_bench.rs`) | about 20 to 180 ns for expressions on context values; 0.5 to 0.9 µs with document reads; compile 0.3 to 2.7 µs |

## P6: compiled frame expressions (2026-10-19)

`Expression::parse` compiles the validated syntax tree into bounded bytecode.
Input names, helper lookup and syntactic Math callees are resolved once. Jumps
preserve conditional/short-circuit reads; calls evaluate arguments left to right
without evaluating their callee, exactly as before. Coercion, member access,
helper formulas and V8 math remain the same routines. The compiler computes the
maximum stack; up to eight live values and twelve numeric arguments stay inline.
The larger bounded cases spill through `smallvec`.

The default `evaluate` still records the exact dynamic dependencies. The new
`evaluate_untracked` avoids path allocations for hosts that invalidate every
document revision or track their reads themselves. pm-wasm uses it: pm-eval
invalidates expression memos on document changes, and detached queries retain
pm-model's read tracking. Literal host keys are converted to pm-model strings
once per source; dynamic keys keep the ordinary conversion. Legacy-dialect
expressions do not compute the inaccessible native `frame` input, or construct
unused `ch` strings.

Interned pm-eval plans retain the compiled source for both static and keyed
bases, so hundreds of distinct resident expressions do not churn the shared
256-source cache every frame. Nonempty keyframes still use the original exact
keyframe evaluator. Memo keys remain revision plus the original f64 time bits;
there is no quantization or tolerance. The 400-source keyed regression checks
that four frames compile exactly 400 times.

Renderer batching and paired timing evidence are documented in `perf.md` under
P6/B3. The parity fixture additionally compares tracked and untracked outcomes,
including encoded number bits and exception messages, for every compiled case.

The second P6 pass adds a typed numeric plan ahead of the complete bytecode.
Runtime types that need coercion decline that plan and use the complete native
bytecode. Document fields, params, fps and idx can be bound at an interned slot's
revision; time, local time, frame and keyed values stay dynamic. Every root
revision discards bindings, including unrelated edits, so no document read is
assumed constant across a write. Bound plans retain the original bytecode for
runtime type changes. Parity now compares unbound and bound numeric results,
including changed time/frame/keyed-value inputs, against the same exact V8
vectors. The same-revision slot also reuses its validated layer position; it
resolves the composition once per batch and revalidates after any revision.

The third pass exposes a guarded scalar-input entry on a bound numeric plan.
It accepts local time, composition time and a numeric base only; any remaining
document read or unavailable input declines to the complete evaluator. pm-eval
uses finite answers only, at the exact validated revision, with no outstanding
expression errors. That removes layer/scope resolution, address cloning and
context construction for these warm channels. Non-finite answers, nonnumeric
bases and error recovery retain the ordinary evaluator. The existing V8 vectors
also compare this entry, including changed dynamic inputs; production engine
regressions cover revision changes, numeric-to-string bases and error recovery.
