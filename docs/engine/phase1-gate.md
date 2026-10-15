# Phase 1 gate

The Phase 1 gate (plan section 26, "Phase 1: Evaluation and the write path", with
amendments 16, 19, and 20), measured on `feat/ne-parity`: `feat/native-engine` at
`2b6ac8d3` (wave 3 merged) plus `feat/ne-edit`, the pm-wasm queries and `pm-cli parity`
of this lane, and the e2e spec fixes below. Reference machine (decision 11): M4 Max,
128 GB, macOS 27, shared with other lanes building and measuring at the same time.

> Gate: 2D evaluation parity on the corpus; zero dual-answer mismatches beyond tolerance
> and zero auditor failures across the e2e suite per the ledger; text sub-gate (line
> breaks and advances); native and WASM bit-identical; S5 targets met in the app.

## Summary

| Gate line | Measured | Result |
|---|---|---|
| 2D evaluation parity on the corpus, native (`pm-eval`, CoreText registry) | corpus v1: 1,891,448 values compared, 0 mismatched; 810 tolerance questions (variable-font ink, max 1.19e-4 px against the 1e-6 bound) | fail pending a tolerance decision (T2) |
| 2D evaluation parity on the corpus, through pm-wasm in Node | 1,888,298 compared, 1,260 mismatched (all `system-ui` 24 and 26 px text layouts of the real project), the same 810 tolerance questions | fail pending a decision (T1, T2) |
| Dual answers across the e2e suite | 754,224 compared, 0 mismatched, 0 stale, 0 allowed; 1 fragile case, 0 differed; 338 consistency checks, 0 mismatched | pass |
| Auditor across the e2e suite | 8,214 audits, 0 mirror mismatches, 4 failures: 1 written by a spec (fixed, clean on rerun) and 3 by two app writers that do not announce their writes (T5, T6; fixes verified, not committed: other lanes' files) | fail |
| Text sub-gate: line breaks and advances | S4 font corpus: 5,200 cases, identical breaks, max 0.001 px. Parity corpus: 6,162 layouts, identical breaks; advances bit-identical with the capture renderer's font cache, 1,260 layouts up to 18.2 px off as a fresh renderer (pm-wasm in the app) | pass for line breaks; advances fail pending T1 |
| Native and WASM bit-identical | corpus: 1,815,420 pm-wasm replies, 0 differ (V8); pm-wasm random projects: 636,463 replies, 0 differ; pm-eval digests equal over 243,703 / 148,721 / 91,003 values; pm-text: 5,200 layouts byte-identical (JavaScriptCore); under V8 (Electron 44's Node) as well | pass |
| S5 targets in the app | `evP` cached 89.5 ns (target < 1 µs); structural apply p95 0.4 to 0.7 ms (< 2 ms); property apply p95 0.1 ms in one run and 0.2 ms (the timer's second 0.1 ms tick) in the other, mean 0.067 ms, back to back 0.010 ms (< 0.2 ms) | pass, the property p95 at the timer's resolution |

As measured the gate does not pass. What stands between it and a pass:

1. **T1**, a decision for Jude: how level 1 measures `system-ui` at the sizes the app's
   own UI text puts into Chromium's font cache (fresh renderer, or the app's UI state at
   2x). 1,260 corpus layouts depend on it.
2. **T2**, a decision for Jude: a variable-font ink class in the bounds tolerance (the
   differences are at most 1.19e-4 px), or a port of CoreText's variation arithmetic.
3. **T5 and T6**, two app writers to announce their writes (verified 1-line and 3-line
   fixes below; Media and Renderer write path lanes).
4. **T3**, pm-eval: amendment 16's integration step (Model and evaluation lane), with a
   corpus case that exercises it.

Nothing here is passed on a looser tolerance than section 25's: values outside the plan's
tolerance are mismatches or, where a looser named tolerance would accept them, tolerance
questions, which are not passes.

## `pm-cli parity eval`

`engine/crates/pm-cli` (`src/parity/`). It reads the newest `v1-*` corpus capture under
`~/Library/Application Support/Powermove Engine Corpus/` (else `v0-*`, or `--corpus`),
evaluates every project at every captured sample time, and compares every value the
evaluation dumps record (`docs/engine/corpus.md`, "Evaluation dumps"):

- per composition state (the open composition, then every active precomp at its source
  time, recursively): the composition record (`id`, `w`, `h`, `fps`);
- per layer: its record (`id`, `name`, `type`, `index`, an extension's `definition`),
  `active`, every channel of `allProps` (`PM.ev` for `p` keys, `PM.evP` otherwise), each
  channel's expression error, the layer fields (`blend`, `mblur`, `on`, `matteMode`
  through `evP` when they are properties; `matteSource`, `parent`, `group`, `threeD`,
  `solo`), resolved content, local and world matrices, world opacity, the 3D plane
  matrix, `GL.bounds`, `PM.textLayout`, `PM.textRasterGeometry`, the text animation
  state and posed glyphs, `PM.captionBounds`, path values and group matrices, and the
  source time of video, precomp, and audio layers (with the engine's integration step
  and, as a separate category, the legacy step);
- per dump: the 3D scene description.

Rules (`src/parity/compare.rs`): numbers within `1e-9 + 1e-9·|b|`; strings, booleans, and
`null` exact; objects by member (order carries no meaning), arrays element by element;
non-finite numbers only equal themselves; bounds, text raster geometry, caption bounds,
and text layouts within 1e-6 absolute. Allowed differences, counted apart: a 180 against
−180 rotation at `atan2`'s branch cut (amendment 20; not for bounds or text) and a
variable-speed source time in a nested composition (amendment 16). A text layer whose
layout differs from the capture is counted once as a `textLayout` mismatch, and the
geometry that depends on it (its bounds and raster, a group's bounds over it) is listed
as not compared. Variable-font ink outside 1e-6 but within 2e-4 is a tolerance
question. A legacy throw (the dump's `*label` errors) must be a throw on our side too;
the messages are compared as their own category.

With `--wasm` every item becomes the pm-wasm queries the renderer would send. Fonts are
served as main serves them: registry answers from the native registry for each query the
mirror reports missing, and the faces they name (whole faces with outlines by default,
extracted from collections; `--wasm-subsets` serves the metrics-and-shaping subsets the
app serves, so ink-dependent values are not compared). A fresh native `pm_wasm::Engine`
and a fresh instance of the built module in Node (`scripts/parity-wasm.mjs`) receive the
same answers and faces and answer every request; the replies must be byte-identical, and
the module's replies are judged against the corpus. pm-wasm gained the queries the
kernel surface lacked (`src/queries.rs`): `glyphLayout`, `textRasterGeometry`,
`textAnimation`, `captionBounds`, `sourceTime`, `compositionScene`, `expressionError`.

```sh
source engine/env.sh
node engine/crates/pm-wasm/scripts/build.mjs           # the module
cd engine && cargo build --release -p pm-cli
target/release/pm-cli parity eval                      # native, capture font cache
target/release/pm-cli parity eval --wasm --json report.json
target/release/pm-cli parity eval --fresh-font-cache   # as a fresh renderer
target/release/pm-cli parity eval --wasm --no-native --wasm-subsets
```

Exit status: 0 when every compared value is within tolerance or allowed and native and
wasm agree; 1 on a mismatch or identity difference; 2 when only tolerance questions
remain. A full run (132 projects, 422 dumps, native and wasm) takes about 40 s.

## Corpus evaluation parity

Corpus v1 (`v1-20261012-230504-ae47a3b3`, captured at `ae47a3b3`, Electron 44, device
pixel ratio 2), 2026-10-14. Columns: values compared and mismatched natively (capture
font cache), then through pm-wasm in Node (whole faces).

| Category | Native compared | Mismatched | Questions | Max relative error | WASM compared | Mismatched | Questions |
|---|---|---|---|---|---|---|---|
| composition record | 471 | 0 | 0 | 0 | 471 | 0 | 0 |
| layer record | 41,414 | 0 | 0 | 0 | 41,414 | 0 | 0 |
| active | 41,414 | 0 | 0 | 0 | 41,414 | 0 | 0 |
| channels | 773,554 | 0 | 0 | 0 | 773,554 | 0 | 0 |
| expression errors | 773,554 | 0 | 0 | 0 | 773,554 | 0 | 0 |
| layerFields | 41,414 | 0 | 0 | 0 | 41,414 | 0 | 0 |
| content | 41,414 | 0 | 0 | 0 | 41,414 | 0 | 0 |
| localMatrix | 41,269 | 0 | 0 | 6.9e-17 | 41,269 | 0 | 0 |
| worldMatrix | 41,269 | 0 | 0 | 2.5e-16 | 41,269 | 0 | 0 |
| worldOpacity | 41,269 | 0 | 0 | 0 | 41,269 | 0 | 0 |
| planeMatrix | 238 | 0 | 0 | 0 | 238 | 0 | 0 |
| bounds | 33,226 | 0 | 0 | 0 | 33,226 | 0 | 0 |
| bounds text | 2,157 | 0 | 0 | 0 | 2,157 | 0 | 0 |
| bounds group | 559 | 0 | 0 | 0 | 559 | 0 | 0 |
| bounds (variable font) | 5,220 | 0 | 405 | 4.3e-6 | 3,330 | 0 | 405 |
| textLayout | 6,162 | 0 | 0 | 0 | 6,162 | 1,260 | 0 |
| textRaster | 2,157 | 0 | 0 | 0 | 2,157 | 0 | 0 |
| textRaster (variable font) | 4,005 | 0 | 405 | 4.3e-6 | 2,745 | 0 | 405 |
| textAnimation | 57 | 0 | 0 | 7.5e-16 | 57 | 0 | 0 |
| captionBounds | 28 | 0 | 0 | 0 | 28 | 0 | 0 |
| paths | 49 | 0 | 0 | 0 | 49 | 0 | 0 |
| sourceTime | 242 | 0 | 0 | 1.4e-16 | 242 | 0 | 0 |
| sourceTime (nested) | 16 | 0 | 0 | 0 | 16 | 0 | 0 |
| sourceTime, legacy step | 258 | 0 | 0 | 0 | 258 | 0 | 0 |
| scene3d | 32 | 0 | 0 | 0 | 32 | 0 | 0 |
| **Total** | **1,891,448** | **0** | **810** | | **1,888,298** | **1,260** | **810** |

No allowed difference occurred (no branch-cut rotation, no differing nested source time),
and no value threw on one side only.

Not compared, both runs: 107 bounds of 3D objects and model groups (three.js answers them
through `LayerHost` in Phase 1; plan section 26, Phase 1), and the 200 `shaderMeta`
entries (the compiled GL program's uniform list and its cache key: shader pipeline
metadata for Phase 3a, not an evaluation value). Through pm-wasm also 1,890 variable-font
bounds and 1,260 variable-font raster geometries, because their layouts are the 1,260
that differ (T1). With `--wasm-subsets` (the app's configuration) every ink-dependent
value is answered by the legacy code instead (pm-eval allowed difference 6): 2,155 text,
2,758 variable-font, and 7 group bounds and 4,902 raster geometries are not compared;
the rest is as above (1,878,476 compared, the same 1,260 mismatched).

Corpus v0 (`v0-20261012-064900-63aea4fd`) gives the same counts in every category, natively
and through pm-wasm. `--fresh-font-cache` natively gives exactly the pm-wasm result
(1,260 mismatched), so the WASM difference is the font cache state alone.

Picks are not in the corpus (the capture records no `GL.pick`); they are compared by
pm-eval's layer vectors (2,592 picks under bun, all agree, `docs/engine/pm-eval-layers.md`)
and by dual answers across the e2e suite. The fragile class of amendment 20 needs the
legacy answer under perturbation, which the corpus does not record; no bounds case
mismatched, so it is not needed here.

## Triage

| # | Values | Class | Finding |
|---|---|---|---|
| T1 | 1,260 text layouts (`system-ui` at 24 and 26 px, the real project; their 1,890 bounds and 1,260 raster geometries follow) | Legacy defect, decision for Jude | Chromium's font cache keys platform data by effective size, so the app's own UI text (`system-ui` at 12 and 13 px on a 2x display) fixes `opsz` for every later 24 and 26 px canvas measurement in that renderer (`docs/engine/text.md`, "The renderer's font cache"). pm-text reproduces the capture bit for bit when told that state (`FontCacheSeed`); pm-wasm has no seeds and measures as a fresh renderer, so in the app its layouts differ from Canvas2D's by about 3.7% per glyph (up to 18.2 px at a line's end). Line breaks are identical. Today's answer depends on the display and on what the UI drew first; the open question in text.md (fresh renderer, or the app's own UI state at 2x) decides whether the engine models the seeds (a `pm_fonts_seeds` export fed from main) or the gate accepts the fresh-renderer answer |
| T2 | 810 values: 405 text bounds and 405 text raster selections in a variable font, all in the real project | Tolerance question | CoreText instances variable outlines with its own variation arithmetic; skrifa's differ by up to 1.19e-4 px (relative 4.3e-6), above section 25's 1e-6 for bounds. pm-eval's own corpus test accepts 2e-4 for these (`VARIABLE_INK`). Either the bounds tolerance gains a variable-font ink class or pm-text ports CoreText's variation arithmetic |
| T3 | none in the corpus | Engine bug (pm-eval) | Amendment 16 is not implemented: `Retiming::Engine` (`pm-eval/src/retime.rs`) integrates variable speed with one midpoint sample per frame of the owning composition, not two at no less than 60 Hz, so a root composition does not match today exactly. With an eased speed (0.5 to 3 over 1.5 s, influence 80%) at 30 fps the engine answers 0.156383 s at t = 0.3 against legacy 0.156402 (2e-5 s off; 2e-4 at t = 0.7). The corpus does not catch it: its only variable speed (`retiming/video`) is a linear ramp, which the midpoint rule integrates exactly at any step. Needs the pm-eval fix and a corpus case with eased and expression speed at the root and nested |
| T4 | 107 bounds, 200 `shaderMeta` | Out of Phase 1 scope | Listed above |
| T5 | 1 auditor missed write (`import-svg.spec.ts:7`, `assets.<id>.format`) | Engine bug (write path; `legacy/gl/raster.ts`, media) | `PM.assets.restoreProject` hydrates `format` on an older project's asset record (`meta.format = a.format`, raster.ts:1339) with no announcement. The fix below announces it with `PM.touch()` when it hydrated anything |
| T6 | 2 auditor stale answers (`media-store.spec.ts:97`, `:177`, `requiresEngine`) | Engine bug (write path; `media/host-media-store.ts`, Media lane) | The `requiresEngine` stamp is set and removed by a bus listener (`stamp()` in `installHostMediaStore`) after the barrier marked the announcement; a query from the viewer overlay then answers from a mirror without the stamp. The fix announces the path with `PM.doc.touched([['requiresEngine']])` |
| T7 | `native-engine.spec.ts:30` failed | Environment | The worktree had no `apps/desktop/native/pm_node.node` (`bun run build:engine`); with it built the test passes |

## Dual answers and the auditor across the e2e suite

`VITE_POWERMOVE_NATIVE_EVALUATOR=audit` (mirror, routed queries, dual answers, the
auditor every 100 ms), `POWERMOVE_ENGINE_FONTS=1`, `POWERMOVE_NATIVE_REPORT_FILE`,
`playwright test --workers=1 --retries=0` over the whole suite (quarantined tests
included), one Electron at a time under the machine lock, 2026-10-14: the app code of
`0d21f524` (the later commits before the run change only engine crates) with the pm-wasm
module built from `801374ab`.
Summed with `node apps/desktop/scripts/native-evaluator-report.mjs <report> --results
<playwright json>`.

| | |
|---|---|
| Tests | 559: 510 passed, 28 failed, 21 skipped (environment), 28.3 min |
| Failures | the 27 tests in `e2e/quarantine.json`, all as quarantined, and `native-engine.spec.ts:30` (T7; passes once the addon is built) |
| Launches | 589 pages, 605 mirrors ready |
| Dual answers | 754,224 compared, **0 mismatched**, 0 stale; allowed differences 0; 1 fragile case, 0 differed |
| Consistency checks | 338 before saves, exports, and on idle, **0 mismatched** |
| pm-wasm | 0 refused batches; 7,564 flushes, 1,995 batches, 28,114 operations; tracking violations 0 (app writers 0) |
| Auditor | 8,214 audits, 0 mirror mismatches; **4 failures**: 2 missed writes, 2 stale answers |
| Fallbacks to the legacy answer | `gl.bounds` 1,510 for fonts not served yet and 3,389 for ink (subsets carry no outlines), `gl.pick` 37 ink, 42 3D, 6 fonts, `gl.bounds` 2 host, raster metrics 23 fonts and 191 ink, text layout 23 fonts, `hasKeyAt` 54 detached properties; these are answered by the legacy code by design and are not compared |

The 4 auditor failures:

| Test | Path | Writer | Status |
|---|---|---|---|
| `video-frame-integrity.spec.ts:33` | `comps.C.layers.N.mblur` | the spec (`PM.proj.layers[0].mblur = true`) | fixed in the spec (`PM.touch()`); the file's three tests rerun with 0 auditor failures |
| `import-svg.spec.ts:7` | `assets.<id>.format` | app: `restoreProject` (T5) | open |
| `media-store.spec.ts:97`, `:177` | `requiresEngine` | app: the engine stamp listener (T6) | open |

The wasm lane's run of the same suite (2026-10-14, before this branch) had 9 missed
writes and 3 stale answers, all from specs writing `PM.proj` with no announcement; this
lane added `PM.touch()` after each such write (listed below), and none of them recurs.
T5 and T6 are app writers. Both fixes were applied to a scratch tree and verified with
`import-svg.spec.ts` and `media-store.spec.ts` in `audit` mode (9 tests passed, 258
audits, 0 failures, 9,301 dual answers, 0 mismatched), then reverted, because the files
belong to other lanes:

```diff
--- a/apps/desktop/src/renderer/src/legacy/gl/raster.ts
+++ b/apps/desktop/src/renderer/src/legacy/gl/raster.ts
@@ restoreProject
+    let hydratedFormat = false;
     restored.forEach((a: any) => {
       const meta: any = project.assets?.[a.id];
-      if (a.format && meta && meta.format !== a.format) meta.format = a.format;
+      if (a.format && meta && meta.format !== a.format) { meta.format = a.format; hydratedFormat = true; }
     });
+    if (hydratedFormat) PM.touch();
--- a/apps/desktop/src/renderer/src/media/host-media-store.ts
+++ b/apps/desktop/src/renderer/src/media/host-media-store.ts
@@ installHostMediaStore
     if (!project?.id || !applyEngineStamp(project)) return;
+    PM.doc?.touched?.([['requiresEngine']]);
```

With both applied, the suite's auditor failures from this run would be 0; the full suite
was not rerun with them.

## Text sub-gate

Line breaks and advances (section 25 text class, level 1: identical line breaks and
advances within 0.25 px):

- **S4 font corpus** (`cargo test -p pm-text --test corpus`, 2026-10-14): 5,200 cases
  against Canvas2D (Chrome for Testing), identical breaks in all 5,200, all within
  0.25 px, max error 0.001047 px (p99 0.000172). The amendment 19 fonts (Blink's effective
  size) are part of it.
- **Parity corpus** (`pm-cli parity eval`): 6,162 text layouts, 74,032 units. Line breaks
  identical in all. With the capture renderer's font cache every position and advance is
  bit-identical; as a fresh renderer (pm-wasm) 1,260 layouts have advances over 0.25 px
  (T1), max 18.2 px of accumulated position.
- **Amendment 19's pins**: no font has text layouts left out of the comparison (pm-eval's
  `PINNED_TEXT_DIFFERENCES` is empty; `pm-cli` counts every differing layout as a
  mismatch).

## Native and WASM bit-identity

| Check | Measured |
|---|---|
| `pm-cli parity eval --wasm`, corpus v1 and v0 | 1,815,420 replies each, 0 differ (Node 24, V8); whole faces and subsets |
| `node engine/crates/pm-wasm/scripts/identity.mjs --seeds 20 --layers 60` | 636,463 replies (about 2.6 million numbers), 0 differ |
| `bun engine/crates/pm-eval/examples/support/check-wasm-identity.ts` | digests equal: evaluation 243,703 values (`74dd9610e4ebff84`), layers 148,721 (`97882587631478d0`), captions 91,003 (`85f7dbc79ed55c85`) |
| `pm-text` `scripts/wasm-identity.mjs` (from `scripts/check.sh`) | 5,200 layouts byte-identical to the native subset run and identical to the native CoreText run under Bun (JavaScriptCore); level 2 byte-identical; the same under V8 (Node 24.18.1 in Electron 44, `ELECTRON_RUN_AS_NODE`), 5,200 of 5,200 |

## S5 targets in the app

`node apps/desktop/scripts/pm-wasm-bench.mjs --rounds 200`: the built editor (hidden,
unfocusable), flag `on`, S5's 884-layer project (15,858 channels, 66 animated, 1.19 MB of
JSON), edits through `PM.Edit.apply` and `PM.hist.do`, flushed through the write barrier.
Electron 44, M4 Max, 2026-10-14, two runs under the machine lock while other lanes built
and measured; `performance.now()` resolves 0.1 ms here, so per-edit p95 values are timer
ticks.

| Measurement | S5 target | Run 1 | Run 2 | |
|---|---|---|---|---|
| `evP` cached, all channels (identity registry) | < 1 µs | 89.5 ns (legacy 29.0) | 89.5 ns (legacy 29.3) | pass |
| `evP` cached, animated channels | < 1 µs | 60.6 ns | 60.6 ns | pass |
| `evP` at a new time, animated | - | 152 ns | 182 ns | - |
| Fast path batch, per value | - | 10.8 ns | 10.8 ns | - |
| Property edit: pm-wasm apply, p50 / p95 / mean | < 0.2 ms | 0.1 / 0.2 / 0.067 ms | 0.1 / 0.1 / 0.0675 ms | pass at the timer's resolution |
| Property edit with its path, mean of 200 back to back | < 0.2 ms | 0.0105 ms | 0.010 ms | pass |
| Property edit through `Edit.apply`: flush (diff + encode + apply), p50 / p95 | - | 0.4 / 0.5 ms | 0.4 / 0.5 ms | - |
| Structural reorder / insert / delete: pm-wasm apply, p95 | < 2 ms | 0.7 / 0.4 / 0.4 ms | 0.6 / 0.5 / 0.4 ms | pass |
| Structural reorder / insert / delete: flush, p95 | - | 6.1 / 4.4 / 4.7 ms | 4.8 / 5.2 / 4.5 ms | see below |
| Load (resync of the whole project) | - | 47.5 ms | 46.2 ms | - |
| Consistency check | - | 37.1 ms, ok | 36.8 ms, ok | - |
| pm-wasm linear memory / heap live / heap peak | - | 39.9 / 25.8 / 27.2 MB | same | - |

The S5 targets concern pm-wasm's apply, and they are met. As the write-path notes record
(`docs/engine/write-path.md`, "S5 targets in the app"), a structural flush end to end
(the barrier's diff of the 884-layer `layers` array plus encode and apply) is above
S5's 2 ms: 4.5 to 6.1 ms p95 here against 3.4 to 3.5 ms in the wasm lane's run on a less
loaded machine. The history commit that produces such an edit costs about 40 ms on this
project, so the barrier adds roughly a tenth to it. Recorded as measured; not an S5 line.

## Changes made for the gate

- `engine/crates/pm-cli`: the `parity eval` subcommand (this lane).
- `engine/crates/pm-wasm/src/queries.rs` (new), two lines in `engine.rs`, one in
  `lib.rs`, `tests/queries.rs` (new): the queries listed above. Owned by the
  renderer write path lane; additive (unknown ops still answer "unknown query").
- `apps/desktop/e2e`: specs that assigned `PM.proj` members outside history now call
  `PM.touch()` right after the write (the write-barrier contract), so the auditor's
  missed writes and stale answers from test code go away:
  `agent-canvas-capture`, `agent-path-mask`, `export-dialog`, `export-video-frames`,
  `import-svg`, `media-replacement`, `project-close-unchanged`, `split-text`,
  `transparent-sequence-scrubbing`, `video-frame-integrity`, `viewer-tools`.
  `export-dialog.spec.ts:5` waits for the squircle clip paths, which
  `settings/squircle.ts` paints only once the element has a layout size.
