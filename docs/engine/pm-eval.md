# pm-eval: the evaluation core

Design notes for `engine/crates/pm-eval` (plan section 7; Phase 1). The crate answers
what the legacy evaluator answers (`legacy/core/anim.ts` and the files it calls) as a
pure function of a `pm_model::Document`, a composition, and a time. This note records
the public API later lanes build on, the cache rules, and every place the engine
deliberately differs from the TypeScript.

## Modules

| Module | Ports | Contents |
|---|---|---|
| `js_sem` | V8 semantics | `JsValue`; `ToNumber`, `ToString`, `ToBoolean`, `typeof`; `+` with concatenation, `- * / %`; loose `==`, `===`, relational comparison; `Math.round` (halves toward +∞, `-0` in [−0.5, −0]); `parseInt`; `Number.prototype.toString(radix)`; `padStart`; `JsThrow` |
| `math` | section 7, determinism | `libm` for every transcendental function; ECMAScript `pow` special cases |
| `node` | property access | `obj[key]` on `pm_model::Node` with JavaScript's rules for arrays, strings, primitives, `null` and `undefined`; `for…in`, `Object.entries`, `for…of`; `isProperty` |
| `temporal` | `core/anim/temporal-ease.ts` | the speed and influence model: control points, `solveCurveParam`, `valueAtTime`, unit-handle conversions, auto-bezier |
| `easing` | `legacy/core/easing.ts` | bezier solver, the 27 presets, `nameOf`, `handles`, the float32 spring table, `springDuration` |
| `keyframes` | `evalKfs` and `temporal-bridge.ts` | track evaluation: hold, linear, bezier, auto-bezier, continuous, springs, per-byte hex color interpolation, string and boolean steps, mixed tracks; `PreparedTrack` |
| `address` | `findProp`, `allProps` | every key namespace (transforms, `c.`, `l.`, effects and `$enabled`, `m.`, `u.`, `x.`, transitions, `g.`, `mp.`, `ta.`, `ts.`, and the 3D namespaces `m.`, `light.`, `camera.`, `environment.`, `shader.`, `slots.<id>.`, `model.`, `o.<id>.`), labels and groups |
| `comp` | `PM.curComp()`, `PM.proj` | `Comp`: one composition of a document revision, the legacy composition object view, rates, ticks to seconds, layer indexes, `groupAncestors` |
| `expr` | `evalProp`'s interface | `ExpressionEngine`, `CompiledExpression`, `ExprContext`, `ExprDeps`, error texts |
| `evaluator` | `PM.ev`, `PM.evP`, `PM.active`, `PM.resolveContent` | `Evaluator`, hooks, interned ids, caches |

## API

### By node

`Evaluator::ev(cx, layer, key, T)`, `ev_p(cx, layer, prop, T, key)`, `active(cx, layer, T)`,
`resolve_content(cx, layer, T)`, `find_prop`, `all_props`, `group_ancestors`. These keep the
legacy signatures over node references; `cx` is a `Comp` (`Comp::resolve(&doc, "compId")`).
Nodes may be detached (`Node::from_value` of `api.model.mkLayer(type)`), so these calls keep
no per-node state: each prepares the track it reads (about 1 µs for a four-key bezier
track). The one revision-keyed cache they read is the composition's layer index (group
ancestry in `active`), reused only for the same `layers` node (see below). Results are
`JsResult<JsValue>`: `Err` wherever the legacy code throws.

### By id (the JS fast path and renderers)

`Evaluator::intern(PropAddress) -> PropId`, `ev_id(doc, id, T)`, `ev_id_ticks(doc, id, ticks)`,
`ev_batch(doc, ids, T, out: &mut [f64], kinds)`, `release(id)`. An address is (composition
id, layer id, key, `Via`):

- `Via::Channel`: `PM.ev(L, key, T)` (`L.p[key]`; a missing channel answers 0);
- `Via::Find`: `PM.evP(L, PM.findProp(L, key), T, key)`;
- `Via::Member(path)`: `PM.evP(L, L[path…], T, key)` (`l.on` is `["on"]`, a content
  channel is `["d", field]`).

The layer is the first layer with that id. `ev_batch` writes numbers into a slice (the
shared `Float64Array` of section 7) and a `ValueKind` per value; a non-number answer is
`Other` (ask `ev_id` for the `JsValue`), a throw is `Threw`. Released ids are reused.

### Time

Methods take composition time in float seconds, as the document and the kernel do.
`Comp::seconds_at(ticks)` maps a tick on the composition's exact grid to the document's own
float (`IngestRate::seconds_of`: `n / fps` for a frame start), so `ev_id_ticks` evaluates a
frame-aligned tick exactly where the legacy evaluator evaluates that frame.
`Comp::frame_at(seconds)` is the integer `frame` expressions see (section 5, rule 9).

### Expressions (`pm-expr`)

`pm-expr` implements `ExpressionEngine::compile(&JsString) -> Option<Arc<dyn CompiledExpression>>`
and depends on `pm-eval` (for `JsValue`, `js_sem`, `math`, `node`), not the reverse.
`pm-eval` owns what surrounds an expression in `evalProp`: it runs only when `expr` is
truthy; a non-string `expr` or a failed compile records `Expression syntax is invalid.`; the
expression sees `t` (local), `time` (`T`), `fps` (`comp.fps || proj.fps` as stored),
`frame`, `value` (the keyframed or static value), `layer`, `comp` (the legacy object view),
`ch`, and `idx` (computed only when `needs_idx`); a finite number, string, or boolean
replaces the value, anything else records `Expression did not return a finite value.`; a
thrown error records its message (or `Expression failed.`). `ExprContext::param(name)` is
`param()`. `CompiledExpression::dependencies` lets an engine declare what it reads; the
evaluator currently treats every expression as reading anything (see below). Compiled
expressions are cached by source (256 entries, oldest first out, as `PM.exprCache`).

### Hooks

`EvalHooks` (set with `Evaluator::with_hooks`): `channel` (take over a channel; consulted
only when `overrides_channels` is true, so the hot path pays nothing), `active` (refine the
legacy answer: precomp time, transitions), `resolve_content` (add to the content after the
declared channels), `extension_labels` (labels for `allProps` from the extension registry).
Hooks receive `&mut dyn PropEval` to evaluate further channels.

## Caches and invalidation

Caches are keyed by node identity (the layer id), input revision (`pm_model` node
revisions), and time; a patch moves the revisions of everything it touched, so applying
patches invalidates exactly the affected ids with no notification.

- **Plans.** Each id keeps a plan: a constant, a prepared temporal track, or "read the
  nodes" (mixed tracks, expressions, throwing reads). At a new document revision a plan is
  kept when the layer is provably the same logical layer (the composition resolved to the
  same node path, `Comp::at_root`; the layer's id is unique in the composition now and
  when last validated; and its `id` member did not change) and either the whole layer is
  unchanged or, for `Channel` and `Member` (object-only paths), the addressed property node
  is unchanged. A node's revision only moves forward, object members never move between
  keys, and array elements only move within their array, so an unchanged revision at the
  same path proves unchanged content. The path check matters for the stored form: its open
  composition lives at the document root, and opening another composition makes the same
  id resolve to the older copy in `comps`, whose nodes are unchanged but are other nodes.
  `Find` paths cross arrays (effects, masks, paths by id) and are kept only when the whole
  layer is unchanged.
- **Last answer.** Each id keeps its last (time, answer) at the current revision. Across
  revisions it survives for non-expression plans when the plan and `layer.from` are
  unchanged; for expression plans only while the document root is unchanged.
- **Layer indexes** (positions by id, used by interned ids and by `active`'s group
  ancestry) are reused only for the same `layers` node: the same document root (address),
  the same path (`Comp::at_root`), the same `layers` node address, and unchanged
  indexed inputs at each position. A newer `layers` subtree revision validates
  the nullish flag and the `id` member's address and revision before reusing
  the index; a value edit leaves them alone, and reconciliation's reorders
  move the id addresses. No layer or id is inferred from a hash. The interned path also checks that the layer at
  the indexed position has the addressed id; a mismatch rebuilds the index and counts in
  `CacheStats::stale_indexes`, which stays zero unless the contract below is broken.
- **Content-keyed caches** (spring tables, compiled expressions) never go stale.

An evaluator follows one document's history; a revision names one state of that document.
The by-id calls reset the per-document caches when they see a lower revision or another root
node (another document, or the same document moved in memory, which only costs a replan).
By-node calls on two live documents never share an index (their roots differ).
`Evaluator::reset` is for replacing a document in place with an unrelated one at the same
revision, the one case the evaluator cannot detect.

Measured (native, M4 Max, release, load average 7 to 9 from other lanes; synthetic
884-layer composition with 7,956 transform channels, 414 animated;
`cargo run --release -p pm-eval --example bench_eval`):

| Measurement | Value |
|---|---|
| `ev_id`, cached | 4.6 to 5.8 ns |
| `ev_id` at a new time, animated (curve solve) | 7.9 to 9.4 ns |
| `ev_batch`, per value, cached | 4.3 to 4.8 ns |
| First batch (plan every id) | 2.9 to 5.3 ms |
| Batch after a one-layer property edit (revalidate every id) | 0.60 ms |
| `ev` by node, uncached, four-key bezier track | 0.94 to 0.97 µs |

The S5 targets (`evP` under 1 µs cached) concern the WASM boundary and are measured by
`pm-wasm`; the native core leaves the same headroom S5 found.

## Allowed differences

The legacy evaluator mutates and caches live objects, so a few answers depend on
evaluation history. The engine is a pure function of the stored JSON and answers as a
freshly loaded project does:

1. **Adoption.** `temporalEvaluate` rewrites legacy `eo`/`ei`/`hold` keys into speed and
   influence in the live document and installs accessors. The engine computes the adopted
   form without writing it; evaluation results are identical. Values passed through
   unevaluated (`resolveContent`'s non-channel members) are the stored form, not the
   adopted one.
2. **Mixed tracks after a numeric past.** A track whose keys were all numbers at an earlier
   evaluation keeps accessor-backed `hold`/`eo`/`ei` in JavaScript after a key becomes a
   string; the engine reads the stored fields.
3. **`temporalKeys` and `null` keys.** On a mixed track with a `null` key before the first
   non-number value, `temporalKeys` throws on the first evaluation per version and not
   afterwards; the engine follows the steady state (no throw).
4. **Spring cache key.** The legacy cache is keyed by the string `damping|stiffness|mass`,
   so `true` and `"true"` share whichever table was built first; the engine keys by the
   coerced numbers.
5. **Object member order.** `pm-model` stores object members sorted, so `allProps` lists
   non-integer member names inside one object in code point order where JavaScript uses
   insertion order. Lookups never depend on it. The parity generator deep-sorts every key
   object before evaluation, so this difference is excluded from the vectors rather than
   measured; the dual-answer harness must compare `allProps` listings as sets per object
   (or pm-model must keep insertion order) until it is resolved.
6. **Prototype members.** Keys such as `constructor` read as `undefined`; JavaScript finds
   `Object.prototype`'s functions.
7. **Expression error identity.** `PM.expressionErrors` is keyed by the channel object; the
   engine keys by (composition, layer id, key). Every channel object reached through the
   same key of layers with the same id shares one entry: two layers sharing an id, and also
   different objects under one key of one layer (for example `PM.active` evaluating
   `L.on` records an error under `l.on`, and a later `evP(L, findProp(L, 'l.on'))` that
   finds no object, where JavaScript looks up nothing, still reports it). Errors are only
   meaningful after a call that reached a channel object; the parity test compares them
   only then.
8. **`comp.comps`.** For the open composition the legacy object's `comps` omits the open
   composition; the canonical document's includes it.

Section 25's evaluation tolerance (`1e-9 + 1e-9·|b|`) applies to transcendental results
(world matrices, wiggle); channel evaluation itself has none and matches bit for bit.

## Verification

- `tests/ts_parity.rs` replays `tests/vectors/eval-vectors.json`, generated by
  `tests/vectors/generate-eval-vectors.ts` from the real TypeScript under bun: every preset
  and random bezier, springs (including malformed configurations), the temporal-ease
  functions, 608 keyframe tracks of nine kinds at about 20 times each (exact key times,
  ±1e-9 and −1e-12 around them, NaN, ±Infinity, −0), and eight compositions with every
  layer kind and key namespace (`ev`, `evP`, `active`, `resolveContent`, `findProp`,
  `allProps`, expression errors), in the stored and the canonical form, by node and by id.
  The checked-in set compares 58,337 numbers, all bit-identical, plus about 45,000
  strings, booleans, `null`s, JSON values, and throws. `PM_EVAL_SCALE=10` gives a 32 MB set
  (552,697 numbers, about 395,000 other values) that also passes bit for bit. The generator bundled for Node (V8) writes
  byte-identical vectors to bun (JavaScriptCore). `PM_EVAL_SEED` draws other random cases;
  seeds 987654, 15869, and 31337 at `PM_EVAL_SCALE=2` also pass bit for bit.
- `tests/js_sem_v8.rs` replays `tests/vectors/js-sem-vectors.json` from Node: every
  operator over 44 values pairwise, `Math.round`, `%`, `toString(radix)`, `parseInt`.
  V8's `parseInt` for radices other than 10 and powers of two skips leading zeros and folds
  32-bit parts with a fused multiply-add on arm64; the port reproduces that (an x64 V8
  rounds twice and can differ in the last bit above 2^53).
- `tests/invalidation.rs`: plans follow patches (edits, reorders, duplicate ids, deletes,
  renames, releases).
- `tests/index_identity.rs`: the stored form's open composition switching between the root
  and its stale `comps` copy, and by-node `active` on two documents at one revision.
- `tests/shadow.rs`: a random patch stream on the stored and the canonical form checks
  every interned id at three times per round, and the by-node `active` of every layer,
  against a fresh evaluator with the same mock expression engine. Batches include leaf and
  keyframe edits, expression edits, layer inserts, deletes, and reorders in every `layers`
  array, switching the open composition, replacing or deleting whole compositions, adding
  and removing the stored form's top-level `layers`, and parameter edits; times repeat and
  some rounds apply nothing or an empty batch, so the last-answer memo is exercised at one
  revision and across revisions. `PM_EVAL_SHADOW=seed:rounds` runs one longer seed. Locally:
  ten seeds, 225,000 rounds per form, about 268,000 patches per form, no differences and no
  stale-index repairs. Disabling the path checks (with the id check), the last-answer rule for
  expressions, or the `layer.from` rule each makes the default seeds fail.
- Native and WASM: `examples/wasm_identity.rs` and `examples/identity_native.rs` digest the
  vector evaluation plus a 220,000-call `libm` sweep; both give `74dd9610e4ebff84` over
  243,703 values (and agree on the 10× set).

## Left to later lanes

- `pm-expr`: the expression parser and interpreter (`random`, `wiggle`, helpers),
  `expressionDiagnostic`, and finer `ExprDeps` so expression answers survive unrelated
  edits.
- Layer evaluation into `EvaluatedScene` (contract 1): transforms and matrices, groups,
  parenting, 2.5D, effects, masks, transitions, precomp time; 3D (`pm-geom3d`), studio
  defaults; editor-helper shims; the `pm-wasm` boundary and identity registry.
- `pm-edit`: the editing commands of `temporal-ease.ts` and `anim.ts` (`setKey`,
  `easyEase`, `setInterpolation`, `applyUnitCurve`, `offset_property`, ...).

## Document-host composition scopes (round 4)

`Comp::in_root` accepts canonical nested member paths for compositions below
another composition's `comps`; the address keeps duplicate ids in separate maps
independent. pm-wasm can also evaluate a replacement-time `compJson` through
`Comp::at_node`, retaining the current project's members and open composition.
Ad-hoc scope queries clear per-document evaluator caches (interned ids remain
valid), because their temporary nodes must never share cached answers with a
later replacement scope at the same document revision.

## P6 resident expression plans (2026-10-19)

Interned static and keyed expressions now retain their compiled source in a
plan. Same-revision frames reuse the validated layer position and resolve each
composition once per batch. `CompiledExpression::bind` optionally binds pure
document inputs. The initial implementation discarded bindings on every root
revision; EXPR-1 now validates their fixed inputs and refreshes changed bindings
only (see `perf.md`, round 3). Time, local time,
frame and the keyed base stay dynamic. Mixed/keyed bases still use the existing
keyframe evaluator. Last-answer keys remain the exact f64 bits and revision.
`expression_error_at` accepts the address's original JS strings, returning at
once for an empty error map without allocating UTF-8 strings per channel.
The 400-source regression, invalidation vectors and native/wasm V8 comparisons
cover this path; measurements and remaining gates are in `perf.md`.

`binding_matches` defaults to unknown for extension expression engines, so
their bindings are conservatively refreshed. Production numeric plans compare
all fixed input bits and retain the original source separately for rebinding.
Nonnumeric fixed leaves decline binding; general bytecode keeps fresh document
reads and invalidates its answer memo on root edits. Unchanged layer lists
preserve validated positions, and changed lists still check unique first IDs.
Immutable composition views carry unique tokens for sharing validation within
a batch without trusting pointer reuse across documents. Source/shape pools
hold weak plan references, so cache eviction does not invalidate resident IDs.

`document_patched` can advance a bound layer-local slot from the exact applied
base revision when patch paths prove its layer/index unchanged: parameter and
provenance writes, or leaf writes inside another layer excluding its ID.
Structural operations, layer replacements, unknown composition paths and slots
older than that base keep ordinary validation. The notification is called only
after successful document apply. Regressions cover an unevaluated layer edit
followed by a parameter edit and another layer becoming the first duplicate ID.
