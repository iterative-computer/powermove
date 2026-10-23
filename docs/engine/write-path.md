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
| `on` | yes | no | no | no |
| `audit` | yes | yes | yes | no |
| `audit-proxy` | yes | yes | yes | yes, violations reported |
| `strict` | yes | yes | yes | yes, violations throw |

## pm-wasm

A plain C ABI (`src/abi.rs`) over one `Engine` per instance, no generated bindings:
the loader (`engine-doc/wasm.ts`) is small and the same module runs in Electron's
renderer and in Node. Built by `node engine/crates/pm-wasm/scripts/build.mjs`
(`bun run build:wasm` in `apps/desktop`; release, custom sections stripped: 2.89 MB,
0.99 MB gzipped) into `apps/desktop/src/renderer/public/engine/` (gitignored), which
`electron-vite build` copies into `out/renderer/engine/`. The app build does not run
it: a build without the module answers with the legacy evaluator (the loader reports
the failure), and the release workflow does not build it yet; the renderer fetches
`engine/pm_wasm.wasm`, the headless host reads `out/renderer/engine/pm_wasm.wasm` or
`POWERMOVE_PM_WASM`. The only import is `pm_expr.pow` (`Math.pow` of the page, as
docs/engine/expressions.md asks).

- **Document**: the canonical project form. `pm_resync` takes canonical JSON;
  `pm_apply` takes a `canonicalV1` body with its base revision and returns 0 (applied),
  1 (gap), 2 (bad revision), 3 (malformed), or 4 (diverged; earlier operations applied).
  Any non-zero code makes the renderer resync. `pm_hash` is the canonical hash.
- **Fast path** (S5): `pm_intern` turns `{comp, layer, key, via}` into a numeric id;
  `pm_ev(id, t)` returns a number (NaN plus `pm_kind` for anything else);
  `pm_ev_batch` reads ids and writes numbers and kinds through typed-array views;
  `pm_ev_batch_range(start, n, t)` evaluates any run of the same buffers in place.
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

A `touch()` without paths compares every layer of the open composition. Each
id-keyed array element found equal remembers the live content it matched, as one
flat list of keys and primitive values (`JsonMemo`). While that shadow element is
unchanged, a live value that walks the same list serializes to the same JSON, so the
comparison reads only the live side, without keyed lookups on the shadow. Invariant:
the barrier forgets every shadow object along an operation's path before applying it
(the shadow is mutated in place). Values the list cannot vouch for (`toJSON`, other
prototypes, `NaN`, changed shared prototypes) take the exact comparison.

How writes reach the barrier:

| Source | How |
|---|---|
| History commits | `PM.doc.touched(canonicalPaths)` before the entry is announced (listeners of `history` and `history:project-patch` ask at once: the viewer redraws the pen tool's overlay there, after the transaction closed), then `PM.touch(canonicalPaths, true)` |
| Scoped transactions (gestures, `Edit.apply`) | `hist.track(paths)` hands its paths over before the writes |
| Undo, redo, scoped rollback | `PM.doc.write(paths, fn)` around the restore |
| `touch()` without paths | a structural diff of the open composition, the composition the editor has entered (`PM.curComp()`), compositions added or removed, and every project member except `comps` (in every mode, the tracking ones included: writers announce writes made through references the Proxy never handed out) |
| Bus announcements (`layers`, `project`, `assets`, `comps`) from writers that skip `touch()` | treated as a `touch()` without paths, marked by a wrapper around `PM.bus.emit` before any listener runs (the kernel turns them into `project:changed`, and the viewer asks from there; inside an Edit's scoped transaction the held paths are marked again first) |
| `PM.ProjectIndex.invalidate()` | the registry's "the layer structure changed" signal (code that rewrites `layers` outside history must call it, or the index answers with the old layers); treated as a `touch()` without paths, marked before the index is cleared. Code that assigns `layers`, invalidates the index, and selects (which redraws the overlay) before its `touch()` gets fresh answers |
| Queries inside an open history transaction | the transaction's writes reach the barrier at its commit, and code inside it may ask before then (the pen tool between `begin` and its first `touch()`, a split that groups and selects). The first flush of each synchronous run, and the first after an announcement without paths, diffs what the transaction may have written: a plain transaction's open composition (as `touch()` does), a scoped one's tracked paths. Announcements without paths inside a plain transaction wait for that flush or the commit, whose paths replace the structural diff. A write after a query in the same run, with no announcement before the next query, is the one case left to the auditor |
| Legacy key adoption | the adopted keys' path, found by identity anywhere in the document (`PM.doc.wroteObject`); keys outside the document are not a document write |
| Compatibility accessors (`ui-state.ts` `defineCompat`) that take a stored member out of the JSON (an effect's `open`) | `PM.doc.wroteObject(object)` |
| Legacy key adoption during evaluation (`temporal-bridge.ts`) | runs inside a barrier writer scope (`setEvaluationWriter`); a rewrite the JSON can see names the keys (below) |
| `PM.proj = another project` | resync at the next flush |
| `PM.proj = another version of this project` | a whole-document diff |
| Writers that know their paths | `PM.doc.write(paths, fn)` (paths marked before `fn` runs, so a listener asking inside it sees them, and again after) / `PM.doc.touched(paths)` |
| Development and test (`audit-proxy`, `strict`) | the tracking Proxy records every write through `PM.proj` |

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
writers announce those (`touch()`, history), and the auditor is the net for the rest.

Identity. A Proxy is a second identity for the same object, so the Proxy keeps
three rules that the e2e suite found necessary:

- An object that enters the document through a proxy (a new mask pushed onto
  `layer.masks`) is handed out raw until the current task ends. The inserting code
  holds the raw object and compares it with what it reads back
  (`layer.masks.find(m => m.path === path)`); later tasks read it through the Proxy.
- `PM.proj = next` deep-unwraps `next`: new documents are built from reads of the
  current one (`rotateProject({ ...PM.proj })` when a composition opens).
- A proxy stored into the document through a raw reference is replaced by its target
  when it is next read, so the raw document never holds a proxy and saves can clone it.

`===` between a raw reference held across tasks and a later read can still differ;
that is a limit of the tracking modes, never of production. The full e2e suite in
`audit-proxy` (2026-10-13) fails 8 tests that pass in `audit`, all from that limit:
a test that holds a layer across an `await` and asks for its index
(`expression-csp.spec.ts:3`, `idx` reads 0), keyframe and graph drags whose hit
targets and later reads are different identities (`timeline-keyframe-drag.spec.ts:4`
twice, `graph-picking.spec.ts:131` and `:172`, `graph-reference.spec.ts:3`), an
imported SVG's asset record (`import-svg.spec.ts:7`), and an image-sequence export
capture (`import-image-sequence.spec.ts:7`, 12 fps). Because the Proxy changes
behavior, it is opt-in: `on` never installs it, development builds included.

Violations are attributed to the writing code. A violation whose stack contains a
Playwright `page.evaluate` frame is the test's (`page.evaluate`, or
`page.evaluate via <function>` when a test calls an app function directly outside
the transaction the app's own callers open); `native-evaluator-report.mjs` counts
the others as app-writer violations and groups violations by writer.

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
  An id names the document's layer with that id, so once the revision moves it
  answers only for a layer object that is still in its composition's `layers`
  (and that list still the composition's). Undo, Redo, and a scoped rollback
  replace a restored layer by a clone without changing the shape; the replaced
  object is evaluated from its own JSON, as the legacy code reads it, never half
  from the document and half from itself.
- **Answers are live objects**: `findProp` and `allProps` return the layer's own
  channel objects, `pick` the live layer, `hasKeyAt` the live key,
  `groups.ancestors`/`transformRoots` live layers (a memoized answer is served
  only while its layers are still the composition's). `expressionErrors` (the kernel's
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
- **Fallbacks** (counted per operation and reason): the module not ready, three.js
  questions (3D objects, model groups, backgroundless extension scenes), text layout
  before fonts are served, and ink without outlines. Detached channels, parents, and
  paths are no longer fallbacks: they go to pm-wasm as JSON (below).

## Named failures and exact answers ("Use New Engine" on)

`installEngineDoc({ strict: true })` (the headless document host with the new engine
on; docs/engine/document-host.md):

- **Traps.** The compiled module is kept; a trapped instance is replaced
  synchronously (`new WebAssembly.Instance`, far below Chromium's synchronous limit),
  resynced from the document, and the question that trapped is asked again on it, so
  the answer stays native (queries in `native.ts`, kernels in `edits.ts`). A second
  trap on the same question is a `pm-wasm-trap` failure. The 1 s revive timer uses
  the same synchronous path for traps outside a query.
- **Detached inputs** are evaluated ad hoc: `hasKeyAt` takes the channel's JSON
  (`prop`), `transformParentMatrix` and `parent3D` a parent layer's (`parent`),
  `pathValues` and `groupMatrix` a path's (`pathJson`; `groupMatrix` appends it after
  the layer's paths, so parent lookups and cycle closing match the legacy recursion).
  A non-object where a channel or path belongs throws the legacy TypeError natively
  (`hasKeyAt`: reading `kf` or `find`; `pathValues`: `path.id` for the default prefix
  or `path.p`; `groupMatrix`: `path.id`; a primitive: `Object.entries`), before the
  layer is read, as today.
- **Named failures** (`strict` only): a module that does not load
  (`pm-wasm-unavailable`, `retry()` loads it again), a pm-wasm error reply
  (`pm-wasm-error`), a trap twice (`pm-wasm-trap`), and a font provider given up
  after three failures (`font-service-unavailable`). Each is reported once per code
  and message: `engine:failure` on the bus, a `failure` report line, and
  `PM.EngineDoc.failures()`.
- **Font state per instance.** The loader keeps loaded faces per module instance, so
  a replacement instance loads them again. `FontProvider.whole` makes ink questions
  native: the headless document host has it (pm-bridge `FontRegistry.whole`); the
  editor's provider through main does not yet.
- **The editor's boot** (`legacy/bootstrap.ts`, new engine on): the project opens
  once the mirror is ready (bounded wait), and the boot project's faces load first
  (`EngineDoc.warmFonts`, which asks pm-wasm directly, uncounted); every project
  opened later warms its faces. What still answers with the legacy code in the
  editor: a face first used after a project opened (`fonts`, until the fonts lane's
  synchronous channel), ink (`ink`, same), and `context` (mid-replacement, or a
  nested composition held in a composition's own `comps`).
- **Anchor bounds key order.** pm-edit's JSON objects come out with sorted keys;
  `legacyKeyOrder` writes `fontAnchorBounds`/`sizeAnchorBounds` as the legacy edits
  do (x0, y0, x1, y1), so the document's bytes do not depend on which side measured.
- **No headless reference.** In a host without a raster (`headless`), the dual modes
  skip what has no legacy answer there: `gl.bounds`, `gl.pick`, `glyphLayout` (the
  native `PM.textLayout`), and the kernels `captureFontAnchor` and `anchorReference`.
- **Development modes.** `audit`, `audit-proxy`, and `strict` come from the build
  flag (or a development build's stored override); a production build's stored
  override can only choose `on` or `off` (`profileOverride`).

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
  (`property-reveal.spec.ts` caught it); members a query answers unevaluated (an
  extension layer's `params`) keep the property's own order. A mismatch that disappears after repairing the
  mirror is a **stale answer** (a barrier gap, reported as `stale-answer` with the
  query's call site, `page.evaluate: …` when a test evaluated it), not an
  evaluation mismatch. Stale answers count as auditor failures.
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
path, applicable at any revision, and only a resync (another project, a refused
batch, a mismatch) carries it. Another version of the same project assigned to
`PM.proj` becomes an incremental `structuralDiff` batch. When a trapped instance is replaced, the new
barrier continues the revisions, so receivers never see them move backward. The
tracer forwards the same batches to the interactive engine.

## The headless document host

One worker process per open project (docs/engine/document-host.md): each installs
the mirror after history (on by default; required with the new engine on). With no
compositor, `PM.GL.bounds` and `PM.GL.pick` come from pm-wasm alone (2D; `null`
where three.js would be needed), and dual answers skip them there (no legacy
reference exists headlessly).

## The frame-rate validator (`apps/desktop/src/shared/frame-rate.ts`)

One validator for the settings UI, `set_composition`, composition settings, new
compositions (New Composition, Pre-compose, a composition from an asset), load and
validation, Home's media matching, and MCP `create_project` (section 5, rule 1). A new
composition that inherits its rate keeps the source's `fpsRational` (validated as a
document rate); a typed one is coerced and validated as new. It
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
| pm-wasm engine | `cargo test -p pm-wasm --release` | 7 tests, including 2,000 rounds of corrupted, truncated, and random patches and queries (errors, never traps) |
| Native vs wasm32 identity | `node engine/crates/pm-wasm/scripts/identity.mjs --seeds 20 --layers 60` | 636,463 replies (about 2.6 million numbers), 0 differ |
| Barrier, mirror, native API vs legacy | `vitest run src/renderer/src/engine-doc` | random projects: every routed query, zero dual mismatches; mirror hash equals the live document through 60 edits, undos, redos, and structural changes |
| JSON diff vs `diffOps` | `json-diff.test.ts` | 2,000 random pairs identical; the allocation-free comparison of retained elements agrees with the full one on every exception (`toJSON`, wrappers, accessors, absent members) |
| Frame rates vs pm-time | `frame-rate.test.ts` | 3,000+ inputs identical |
| Headless host | `vitest run src/engine` | mirror and 2D geometry without a renderer; per-project workers (docs/engine/document-host.md) |
| Exact answers, named failures | `vitest run src/renderer/src/engine-doc/strict.test.ts` | detached inputs native with 0 dual mismatches; traps answered by a fresh instance; failures named |

## S5 targets in the app

`node apps/desktop/scripts/pm-wasm-bench.mjs --rounds 200`: the built editor
(hidden, unfocusable), flag `on`, S5's 884-layer project (15,858 channels, 1.19 MB of
JSON), edits through `PM.Edit.apply` and `PM.hist.do`, flushed through the barrier.
Electron 44, M4 Max, 2026-10-14, other lanes building at the same time;
`performance.now()` resolves 0.1 ms here, so per-edit p95 values are timer ticks.

| Measurement | S5 target | Measured | |
|---|---|---|---|
| `evP` cached, all channels (identity registry) | < 1 µs | 57 ns (legacy 20 ns) | pass |
| `evP` cached, animated channels | < 1 µs | 45 ns | pass |
| `evP` at a new time, animated | - | 121 ns | - |
| Fast path batch, per value | - | 6.9 ns | - |
| Property edit: pm-wasm apply, p95 | < 0.2 ms | 0.1 ms (tick; mean 0.056) | pass |
| Property edit through `Edit.apply`: flush (diff + encode + apply), p50 / p95 | - | 0.3 / 0.4 ms | - |
| Property edit with its path, flush, mean of 200 back to back | < 0.2 ms | 0.0075 ms | pass |
| Structural reorder / insert / delete: pm-wasm apply, p95 | < 2 ms | 0.5 / 0.3 / 0.3 ms | pass |
| Structural reorder / insert / delete: flush (diff + encode + apply), p95 | - | 3.4 / 3.4 / 3.5 ms | see below |
| Load (resync of the whole project) | - | 50 ms | - |
| Consistency check (flush, canonical hash of `PM.proj`, compare) | - | 25 ms | - |
| pm-wasm linear memory / heap live / heap peak | - | 39.9 / 25.8 / 27.2 MB | - |

Every batch reaches pm-wasm as id-keyed operations, never as a stringified array, so
S5's condition holds and pm-wasm's apply meets the targets. The flush adds the diff
that finds those operations. A property edit through `Edit.apply` names `revision`,
`edits`, and the layer, and comparing the edit log's retained entries dominates its
0.3 ms. A structural edit names the whole `layers` array, so the diff compares each
of the 884 retained layers with the shadow: about 1.6 ms of the 3.4 ms, after the
allocation-free comparison took the p95 down from 6.2 ms. Read end to end, the
structural flush misses S5's 2 ms. The history commit that produces the edit costs
far more on the same project (a plain `PM.hist.do` reorder takes 40 ms in Node: two
snapshots, two parses, `diffPatches`, and clones), so the barrier adds about 7% to a
structural edit. Skipping the comparison needs per-element write tracking that
production writers do not provide: a plain transaction that reorders layers and
edits one of them names only `layers`.

## The e2e suite with the flag on

`VITE_POWERMOVE_NATIVE_EVALUATOR=audit` (mirror, routed queries, dual answers, the
auditor every 100 ms), `POWERMOVE_ENGINE_FONTS=1`, `POWERMOVE_NATIVE_REPORT_FILE`,
`playwright test --workers=1 --retries=0`, one Electron at a time, 2026-10-14, at
`feat/ne-wasm` after the wave 3 merge:

| | |
|---|---|
| Tests | 551: 502 passed, 28 failed, 21 skipped (environment) |
| Failures | the 27 tests in `e2e/quarantine.json`, all failing as they do with the flag off, and `export-dialog.spec.ts:5` (below) |
| Launches | 583 pages, 601 mirrors ready |
| Dual answers | 752,940 compared, **0 mismatched**; allowed differences 0; 1 fragile case, 0 differed |
| Consistency checks | 288 before saves, exports, and on idle, **0 mismatched** |
| pm-wasm | 0 refused batches, 0 traps; 7,272 flushes, 1,940 batches, 28,012 operations |
| Auditor | 7,007 audits, 0 mirror mismatches; 9 missed writes and 3 stale answers, every one a test writing `PM.proj` directly with no writer and no announcement (below) |

Missed writes and stale answers, by the spec line that writes:

| Spec | Write |
|---|---|
| `agent-canvas-capture.spec.ts:59` | `PM.proj.backgroundFill = …` |
| `agent-path-mask.spec.ts:18`, `:28`, `:30` | `clip.masks = …` |
| `export-dialog.spec.ts:12` | `PM.proj.w = 1080; PM.proj.h = 1920` |
| `export-video-frames.spec.ts:37` (4 cases) | `layer.from = 0; layer.dur = …` after import |
| `import-svg.spec.ts:21` | `delete asset.format` |
| `video-frame-integrity.spec.ts:26`, `:69`, `:107` | layers, `from`, `dur`, `trim`, `w` |
| `media-replacement.spec.ts:178` (stale `active` in the overlay) | `layer.from = 2; layer.dur = 1; layer.d.trim = .25` |
| `transparent-sequence-scrubbing.spec.ts:38` (stale `resolveContent`, 2 cases) | `layer.d.w = layer.d.h = 128` |

The same kind of write appears in other runs depending on timing (an audit has to
land between the write and the test's next announcement):
`project-close-unchanged.spec.ts:44`, `:66`, `:85` (`PM.proj.name`),
`split-text.spec.ts:76`, `:83` (`lock`, `d.text`), `viewer-tools.spec.ts:121`
(`PM.proj.layers = …`). Production code has no such writer: the writer inventory
classifies every app site, and app-writer violations under the tracking Proxy are 0.
These specs need a `PM.touch()` (or a history transaction) after the write, as the
write-barrier contract asks of any code that edits the document; the spec files
belong to the Parity and QA lane. Earlier runs on this branch had 105 stale answers
(run of 2026-10-13 22:58) and 24 auditor failures (2026-10-14 00:20); bus
announcements marked before listeners and `ProjectIndex.invalidate()` as an
announcement removed every one that app code caused.

`export-dialog.spec.ts:5` reads the select's `clip-path` as soon as
`data-squircle` appears, but `settings/squircle.ts` paints it only once the element
has a layout size (from its `ResizeObserver`). It failed 2 of 10 runs in `audit`,
where dual answers and the auditor load the main thread, and 0 of 12 with the flag
`on` or `off`; the spec should poll for the clip path. `export-code.spec.ts:9`
(the web player's `stopped` flag) failed once under the same load and passed the
other 4 runs.


## Round 4: strict answers and synchronous fonts

With Use New Engine on, every refusal in NativeApi and NativeEdits reports and
throws `EngineFailure`; `fonts`, `ink`, `host`, `3d`, `legacy`, `flush`, `exception`
and missing context never execute the legacy fallback. The main-frame-only
synchronous font channel supplies whole faces on demand. Retry re-enables the
font loader and forgets failed face-cache entries. Font warmup visits nested
compositions and every keyed time, off the UI installation chain.

Composition addresses may identify a nested canonical member path. Replacement
scopes detached from the current project are evaluated as native `compJson`
inputs, with the open project's rate and members retained. See document-host.md
for tests, remaining 3D/cyclic-pose failures and cross-lane edits.

## Round 5: review fixes

- pm-edit answers the cyclic camera/light reparent natively (the legacy aim's
  stale-cache read, docs/engine/pm-edit.md), so the `legacy` reason and pm-wasm's
  `{legacy}` reply are gone.
- `editKernels.align` runs the caller's own code (the strip's `alignCommands`, or a
  fork's) for layers outside the open composition, an empty selection, and the
  kernel's `host`/`fonts`/`ink` refusals: that code is extension code over
  `api.anim`/`api.render`, which answer natively or fail by name. It is counted in
  `NativeEditStats.callers`, not `fallbacks`. Engine faults stay named failures.
- Composition scopes are addressed in O(1) for `proj.comps` members and from a
  verified path cache for nested ones; a root without `compId` is looked up at the
  mirror's top level (no repair per query); explicit layer lists go by their
  composition's address, or as their structural members only. Measurements are in
  document-host.md, round 5.

## P6 expression batch ABI (2026-10-19)

Warm renderer frames evaluate the resident expression ids of a composition in
one `pm_ev_batch` call. `ExpressionFrames` keeps separate copies per composition
because the ABI's typed-array buffers are shared. A first read interns its live
address; subsequent frame reads use the batch. Nonnumeric values/throws retain
the scalar decoder. Revision, scope, module reset and `Object.is(time)` validate
the renderer cache; native memo time keys remain raw f64 bits.

The low seven bits of a batch kind remain `ValueKind`; bit 7 flags an expression
error. Scalar calls also write a stable status word exported by `pm_status_ptr`.
The renderer reads that word from linear memory and fetches an error message only
when flagged. Views are reacquired after memory growth. The resident id registry
uses weak owners and removes released ids; a module reset drops all batches.
The 120-channel integration unit test covers errors/recovery, signed zero,
repeat times, revisions, nonnumeric results, reset and memory growth; a second
case alternates stored composition scopes.

EXPR-1 keeps channel IDs across leaf/shape changes after live-owner and first-ID
validation. Generated scene-parameter controls touch only their owned value
path, retaining unknown-write handling for customized aliases. Applied numeric
plans validate changed fixed inputs; exact patch paths advance unaffected
layer-local plans only from the applied batch's base revision. Source and
structural edit costs remain separately reported in `perf.md`, round 3.

Round 4 makes frame evaluation lazy again. Evaluating every resident id at each
new time paid for inactive and deleted layers (today's evaluator pays only for
what a frame reads). `ExpressionFrames` now orders each composition's members
as frames read them; the first read of a member at a new time evaluates a run
from it with `pm_ev_batch_range`, doubling (4, 8, 16, …) while reads continue
in order and extending only over members read recently (within about two
frames' worth of reads). Non-members use the scalar call and join the members
at the next epoch (an epoch is one time and revision); an epoch that reads the
members in another order reorders them; members unread for 64 epochs leave. A
prepared batch copies its ids into the shared buffer only after another caller
used it, and copies run results by index into its own arrays. Tests: in-order
runs `[4, 8, 16, 32, 60]` for 120 channels, a 10-of-100-layer sliding window
evaluating at most 1.25x the reads, deleted layers no longer evaluated, and
motion-blur sampling at most 2x.
