# The renderer write path and pm-wasm (Phase 1)

Design notes for `engine/crates/pm-wasm` and `apps/desktop/src/renderer/src/engine-doc`
(plan sections 6, 7, and 20; section 26, Phase 1 with amendments 1 and 3; spike S5;
contract 5). The renderer stays the authority for the document and Undo. pm-wasm is
its mirror and its synchronous evaluator, behind the **native evaluator** flag.

## The flag

`VITE_POWERMOVE_NATIVE_EVALUATOR` at build time, overridden per profile by
`localStorage['powermove:native-evaluator']`; `POWERMOVE_NATIVE_EVALUATOR` in the
headless document host (default `on` there).

| Value | Mirror and kernel queries | Auditor | Dual answers | Tracking Proxy |
|---|---|---|---|---|
| `off` (default) | no (`PM.doc` no-ops only) | no | no | no |
| `on` | yes | no | no | development builds |
| `audit` | yes | yes | yes | no |
| `audit-proxy` | yes | yes | yes | yes, violations reported |
| `strict` | yes | yes | yes | yes, violations throw |

## pm-wasm

A plain C ABI (`src/abi.rs`) over one `Engine` per instance, no generated bindings:
the loader (`engine-doc/wasm.ts`) is small and the same module runs in Electron's
renderer and in Node. Built by `node engine/crates/pm-wasm/scripts/build.mjs`
(release, custom sections stripped: 2.89 MB, 0.99 MB gzipped) into
`apps/desktop/src/renderer/public/engine/` (gitignored); the renderer fetches
`engine/pm_wasm.wasm`, the headless host reads `out/renderer/engine/pm_wasm.wasm` or
`POWERMOVE_PM_WASM`. The only import is `pm_expr.pow` (`Math.pow` of the page, as
docs/engine/expressions.md asks).

- **Document**: the canonical project form. `pm_resync` takes canonical JSON;
  `pm_apply` takes a `canonicalV1` body with its base revision and returns 0 (applied),
  1 (gap), 2 (bad revision), 3 (malformed), or 4 (diverged; earlier operations applied).
  Any non-zero code makes the renderer resync. `pm_hash` is the canonical hash.
- **Fast path** (S5): `pm_intern` turns `{comp, layer, key, via}` into a numeric id;
  `pm_ev(id, t)` returns a number (NaN plus `pm_kind` for anything else);
  `pm_ev_batch` reads ids and writes numbers and kinds through typed-array views.
- **Queries**: `pm_query` takes a small JSON request naming the composition
  (`PM.curComp()`), the open composition, the layer (an index, or a detached layer's
  JSON), and the time; the reply is `{ok}`, `{throw}` where the legacy code throws,
  `{host}` for what three.js answers until Phase 3c, or `{fonts}` when text layout
  needs faces main has not served. Numbers JSON cannot carry (`NaN`, `±Infinity`,
  `-0`, `undefined`) are written as `{"\u0000": "NaN"}` markers and revived.
- **Expressions**: `pm-expr` in its legacy dialect reading the mirror's nodes in place
  (`src/expr.rs`); `comp` is the legacy composition object (`Comp::get`), so
  `comp.layers[i] === layer` holds by node identity.
- **No panics at the boundary**: every export validates its input; a trap would kill
  the instance, and the loader then re-instantiates and resyncs.

## The write barrier (`engine-doc/barrier.ts`)

The barrier keeps a **shadow**: the canonical document exactly as pm-wasm holds it
(plain JSON). Writes record dirty paths; **every query flushes first**. A flush diffs
the shadow against the live document at each dirty path, cut at the first array whose
length or id order changed (so structural edits become id-keyed `order`, `insert`,
and `delete` operations, amendment 3), applies the operations to the shadow, encodes
them (`encodePatchOps`), and applies the body to pm-wasm as the next revision. The
same `PatchBatch` (`baseRevision`, `revision`, `origin`, `count`, `body`) goes to
subscribers: the engine sessions of Phase 2.

The diff reads the live side as `JSON.stringify` does (`engine-doc/json-diff.ts`):
`diffOps` tests membership with `Object.hasOwn`, which keeps a stored `eo` that
`temporalKeys` replaced with a non-enumerable accessor; the JSON walker emits exactly
`diffOps`' operations on plain JSON (2,000 random pairs) and converges on live
objects (`NaN`, `-0`, `undefined`, `toJSON`, accessors).

How writes reach the barrier:

| Source | How |
|---|---|
| History commits | `PM.touch(canonicalPaths, true)` after the commit's patches |
| Scoped transactions (gestures, `Edit.apply`) | `hist.track(paths)` hands its paths over before the writes |
| Undo, redo, scoped rollback | `PM.doc.write(paths, fn)` around the restore |
| `touch()` without paths | a structural diff of the open composition and every project member except `comps` |
| Legacy key adoption during evaluation (`temporal-bridge.ts`) | a structural diff (it rewrites keys without a path) |
| `PM.proj = another project` | resync at the next flush |
| `PM.proj = another version of this project` | a whole-document diff |
| Writers that know their paths | `PM.doc.write(paths, fn)` / `PM.doc.touched(paths)` |
| Development and test (`audit-proxy`, `strict`, `on` in dev) | the tracking Proxy records every write through `PM.proj` |

`scripts/engine-writers.mjs` classifies `PM.touch(paths)`, `PM.doc.write`, and
`PM.doc.touched`, and records the `PatchOrigin` every writer class reaches the mirror
with (`origins` in `docs/engine/writers.json`).

### The tracking Proxy (`engine-doc/proxy.ts`)

`PM.proj` becomes an accessor that hands out proxies; every set, delete, and
enumerable `defineProperty` through them records its path (an array's elements and
length record the array). A write while no known writer is active (a history
transaction, `PM.doc.write`, a document replacement, a flush) is a violation:
reported in `audit-proxy`, thrown in `strict`. The document itself never holds a
proxy (values are unwrapped on the way in), `indexOf`/`includes`/`lastIndexOf` unwrap
their argument, and `structuredClone` and every `postMessage` unwrap proxies.
Objects written through a reference obtained outside `PM.proj` are invisible to it;
the auditor is the net for those.

## The identity registry and the native API (`engine-doc/native.ts`)

- **Layers**: `PM.curComp()` names the composition; a live layer maps to its index
  there (cached per mirror revision, checked by identity on use). A layer that is not
  in the composition (`api.model.mkLayer(type)`) is sent as JSON and evaluated ad hoc.
  If the mirror's layer at that index has another id, a structural write bypassed the
  barrier: the mirror is repaired before answering.
- **Channels**: a live channel object maps to its path inside the layer and an
  interned id (the fast path), revalidated on every call by walking the path; a
  `FinalizationRegistry` releases ids when the objects go (S5 finding 2). A channel
  object outside its layer is evaluated from its JSON with the layer as context.
- **Answers are live objects**: `findProp` and `allProps` return the layer's own
  channel objects, `pick` the live layer, `hasKeyAt` the live key,
  `groups.ancestors`/`transformRoots` live layers. `expressionErrors` (the kernel's
  view) holds pm-wasm's errors on the live channel objects it evaluated, and the
  renderer's for the rest.
- **Routed** (kernel/install.ts, kernel/editor-helpers.ts): `anim.ev`, `evP`,
  `active`, `findProp`, `allProps`, `hasKeyAt`, `wouldCycle`, `resolveContent`,
  `worldMatrix`, `localMatrix`, `transformParentMatrix`, `mul`, `expressionErrors`;
  `groups.bounds`, `ancestors`, `transformRoots`, `span`, `expand`; every
  `space3d.*`; `render.gl.bounds`, `render.gl.pick` (2D; compositions with 3D layers
  fall back, since the order follows the editor's view camera), `render.textLayout`,
  `render.raster()` placement metrics of text layers; and the editor helpers
  `resolveContent`, `evaluatedValue`, `pathValues`, `groupMatrix`,
  `validMatteSource`, `is3DLayer`, `planeMatrix`, `planeContains`, `projectPoint`,
  `inversePlane`, `countTextUnits`, `expressionDiagnostic`.
- **Not routed**: edits (`setKey`, `setKeyOn`, `removeKey`, `applyEaseTo`,
  `temporalKeys`, `enableTimeRemap`, factories) wait for `pm-edit`; `anim.version`
  and `touch` stay the renderer's.
- **Fallbacks** (counted per operation and reason): the module not ready or dead,
  three.js questions (3D objects, model groups, backgroundless extension scenes), and
  text layout before fonts are served.

## Text

Text layout runs in pm-wasm over the registry's answers and metrics-and-shaping
subsets (docs/engine/text.md). A query that meets a missing answer or face replies
`{fonts}`; a `FontProvider` (`engine-doc/fonts.ts`) loads what the layout lacked and
the query is asked again: synchronously in the headless document host (pm-bridge's
`FontRegistry` in process, `POWERMOVE_ENGINE_FONTS=1`, `POWERMOVE_PM_BRIDGE`), in the
background in the editor, where the first query keeps the legacy answer (a `fonts`
fallback) and the next ones answer natively. Main registers `engine-fonts:answer` and
`engine-fonts:subset` with `POWERMOVE_ENGINE_FONTS=1` (`setupEngineFonts` in
`main/index.ts`, the same pm-bridge copy the frame bridge loads) and the preload
exposes them as `engineFonts`; without the flag the calls reject and, after three
failures in a row, the loader gives the provider up (text stays legacy). Answers that depend on glyph ink (text and
caption bounds, picks over text, raster placement) reply `{ink}` while the faces are
subsets without outlines (pm-eval allowed difference 6) and keep the legacy answer;
`pm_fonts_outlines(1)` declares whole fonts. Caret layout (`render.textLayout`),
glyph positions, and widths need no outlines.

## Dual answers, the auditor, and the consistency check

- **Dual answers** (`engine-doc/dual.ts`): the native answer is returned; the legacy
  answer is computed and compared with section 25's tolerances (evaluation
  1e-9 + 1e-9·|b|, bounds 1e-6, text 0.25 px), live objects by identity, object
  members and listings in order. pm-eval lists object members sorted (allowed
  difference 5); the native API puts `allProps`, `resolveContent`, and `pathValues`
  back in the document's own order, because the timeline lists rows in that order
  (`property-reveal.spec.ts` caught it). A mismatch that disappears after repairing the
  mirror is a **stale answer** (a barrier gap, reported as `stale-answer`), not an
  evaluation mismatch.
- **Allowed differences** (amendments 16 and 20), counted under `allowed` by
  `<op>:<class>`, never as mismatches:
  - `atan2`: one answer 180 and the other −180 (within the evaluation tolerance),
    the rotation branch cut. Not applied to bounds or text, which have their own
    tolerances.
  - `fragile`: every `gl.bounds`, `groups.bounds`, and `gl.pick` comparison also asks
    the legacy code at the time ±1e-12 s (picks also at the point ±1e-7 px on each
    axis). When one of those answers differs from the legacy answer, the case is
    fragile (`fragile[op]` counts the population); a difference on a fragile case
    counts as `allowed['<op>:fragile']`. `fragileBudget()` checks the 5% line.
  - `nested-retime` (amendment 16): declared, never counted. No routed query
    integrates variable speed: `sourceTime` is not a kernel query, and pm-wasm's
    queries never call `Scope::source_time`.
- **Auditor** (`engine-doc/auditor.ts`): at task boundaries (every 100 ms, backing
  off to about 5% of the time on large projects) the shadow must equal
  `canonicalProject(PM.proj)` under sorted-key serialization, and pm-wasm's hash must
  equal the shadow's. Failures are reported (`missed-write`, `mirror`) and repaired.
- **Production consistency check**: before every Save, autosave, export, and agent
  export (`PM.Export.run`), and on idle after writes (at most every 30 s): flush,
  compare `canonicalProjectHash(PM.proj)` with pm-wasm's hash; on a mismatch list the
  missed paths, resync, emit `engine:consistency-mismatch`, and report.
- Reports are `[pm-native] {json}` lines (each with `at`, wall-clock ms) through the
  host bridge the kernel captured at boot (main's stderr in the desktop app) and the
  console. With `POWERMOVE_NATIVE_REPORT_FILE` set, main also appends them, one JSON
  per line, to that file (`main/log.ts`); test pages emit a summary every 2 s while
  counts change and on `pagehide`, with a per-page `session`.
  `scripts/native-evaluator-report.mjs <file> [--results playwright.json]` sums the
  last summary of every page and attributes events to the test that was running.

## The patch stream for engine sessions

`PM.EngineDoc.subscribe(fn)` delivers every `PatchBatch` pm-wasm applied, in order
(contract 5): `baseRevision`, `revision`, `origin`, `count`, and the `canonicalV1`
body. A receiver starts from `PM.EngineDoc.snapshot()` (flushed; `revision`,
`encoding: 'canonicalJsonV1'`, the canonical JSON with sorted keys, and pm-wasm's hash
at that revision) and applies batches whose `baseRevision` equals its revision. A
batch with origin `load` is a resync: one leaf set of the whole document at the empty
path, applicable at any revision. When a trapped instance is replaced, the new
barrier continues the revisions, so receivers never see them move backward. The
tracer forwards the same batches to the interactive engine.

## The headless document host

`apps/desktop/src/engine/main.ts` installs the mirror after history with the flag on
by default. With no compositor, `PM.GL.bounds` and `PM.GL.pick` come from pm-wasm
alone (2D; `null` where three.js would be needed), so geometry that used to be
unavailable headlessly now answers.

## The frame-rate validator (`apps/desktop/src/shared/frame-rate.ts`)

One validator for the settings UI, `set_composition`, composition settings, load and
validation, Home's media matching, and MCP `create_project` (section 5, rule 1). It
reproduces pm-time's `rate_from_document` (checked through pm-wasm on 3,000+ inputs).
New rates must divide the flick and lie in 1 to 240 fps and are stored as `num / den`;
rates already in documents keep their float. Media rates become the nominal supported
rate (closest frame duration). Every path stores `fps` with `fpsRational`, which is
now a composition field of the canonical form (`COMP_FIELDS` in compositions.ts,
canonical-json.ts, and `pm_model::canonical`), optional like `audioGain`, so it moves
with its composition when another one opens.

## Verification

| Check | Command | Result |
|---|---|---|
| pm-wasm engine | `cargo test -p pm-wasm --release` | 6 tests |
| Native vs wasm32 identity | `node engine/crates/pm-wasm/scripts/identity.mjs --seeds 20 --layers 60` | 636,463 replies (about 2.6 million numbers), 0 differ |
| Barrier, mirror, native API vs legacy | `vitest run src/renderer/src/engine-doc` | random projects: every routed query, zero dual mismatches; mirror hash equals the live document through 60 edits, undos, redos, and structural changes |
| JSON diff vs `diffOps` | `json-diff.test.ts` | 2,000 random pairs identical |
| Frame rates vs pm-time | `frame-rate.test.ts` | 3,000+ inputs identical |
| Headless host | `vitest run src/engine` | mirror and 2D geometry without a renderer |

S5 targets in the app and the e2e run are recorded in the lane report.
