# Gate fixes after the Phase 1 and Phase 2 gates

The correctness defects the Phase 1 gate (`phase1-gate.md`, Triage) and the Phase 2
gate (`phase2-gate.md`, Summary, "Engine-related specs with Use New Engine on", "E2E
ledger") left open: T3, T5, T6, D1, D2, and R1. Each entry gives the defect, its
cause, the fix, and the evidence: a regression test that fails without the fix, and
the gate's own reproduction run again. Branch `feat/ne-correct` on `feat/native-engine`
at `f47b1f8c` (wave 3 with the Phase 2 gate). Measured 2026-10-16 on the reference
machine (M4 Max, macOS 27, Electron 44, AirPods Pro as the output at 160 ms), shared
with other lanes (1-minute load average 10 to 23). Every Electron run was hidden, one
at a time under the machine lock, `--workers=1`, no retries.

| # | Defect | Fix | Evidence |
|---|---|---|---|
| T3 | Variable speed not integrated as amendment 16 says | `pm-eval/src/retime.rs`: two midpoint samples per frame of the owning composition, at least 60 Hz | Engine and legacy bit-identical at the root at 5 rates; the layer vectors check the engine step against the TypeScript; a corpus project that exercises it |
| T5 | `restoreProject` hydrates `format` unannounced | `legacy/gl/raster.ts`: `PM.touch()` when it hydrated anything | Unit test; auditor 0 failures |
| T6 | The `requiresEngine` stamp unannounced | `media/host-media-store.ts`: `PM.doc.touched([['requiresEngine']])` | Unit test; auditor 0 failures |
| D1 | Exports lose the last frames of video clips | `pm-render`: `RenderRequest::video_end`, today's padding per output kind | pm-render test at 10, 24, 25, 30, 60 fps; `export-video-frames.spec.ts:12` 9 of 9 with the flag on |
| D2 | `PM.proj = PM.mkProject(…)` never plays on the engine clock; Play starts about 300 ms late on Bluetooth | Engine connection: lay the viewer out after a document load; presenter: silent compositions on the host clock from Play, the device opened ahead of the first Play | `export-mp4.spec.ts` 2 of 2 and `native-engine-tracer.spec.ts` 10 of 10 with the flag on; vitest; pm-node child-process test |
| R1 | Image-sequence exports fail intermittently with the flag off | `prepareFrame` waits for the viewer's compositor context (`GL.whenReady`) | `import-image-sequence.spec.ts:7` `--repeat-each=12`: 24 of 24 off, 24 of 24 on |

## T3: amendment 16's integration step

**Defect.** `Retiming::Engine` integrated variable speed with one midpoint sample per
frame of the owning composition, on the tick grid. Amendment 16 asks for two midpoint
samples per frame of the owning composition at no less than 60 Hz, which at the root is
today's step (`1 / (max(30, PM.proj.fps) × 2)`, `retiming.ts`). Eased speeds and speeds
with an expression differed from today by about 2e-7 s at 30 fps; the corpus did not
see it because its only variable speed (`retiming/video`) is a linear ramp, which every
midpoint grid integrates exactly.

**Fix.** The gate's verified patch: both modes use the legacy midpoint rule; the engine
takes `fps` from the owning composition (`comp.fps || PM.proj.fps`, `Comp::expression_fps`,
the rate its expressions see), the legacy mode from the open one. At the root they are
the same number, so the engine answers today's bits; a nested composition keeps its
own rate whichever composition is open (amendment 16's one allowed difference). The
tick integration and the `pm_time` import are gone; the cache key is `(mode, step)`.
`docs/engine/pm-eval-layers.md`, "Time", is updated.

**Evidence.**

- `tests/layer_parity.rs` (the layer vectors from the real `retiming.ts` under bun) now
  also checks every root `sourceTime` case with the engine step (`sourceTime (engine
  step)`, 183 cases at 24, 25, 29.97, 30, and 60 fps). Without the fix it fails (for
  example 2.796848612053986 against today's 2.806995154340919, 7.1159778337797235
  against 7.064160311409845 where keys hold); with it all 183 pass within the
  evaluation tolerance, like the legacy step's 183.
- `tests/layers_unit.rs`: `engine_retiming_matches_today_at_the_root` (an eased speed
  with the expression `value * (1 + 0.1 * Math.sin(time * 7))` through `pm-expr`, at 30,
  24, 29.97, 60, and 120 fps, 400 times each on and off the frame grid: engine, legacy,
  and the cached engine answer bit-identical; it checks that the expression changes the
  answer), and `engine_retiming_of_a_nested_composition_is_its_own_rate` (a 24 fps
  composition in a 60 fps open one: the engine answer equals today's with the nested
  composition open, and differs from today's with the 60 fps one open in more than 150
  of 200 times). The existing `engine_retiming_uses_the_owning_composition_rate`
  passes unchanged.
- `cargo test --release -p pm-eval`: every test passes.
- `pm-cli parity eval` (native, corpus v1): 1,891,448 values compared, 0 mismatched, the
  same 810 tolerance questions (T2); `sourceTime` 242 compared with maximum relative
  error 0 (1.4e-16 before), `sourceTime (nested)` 16, `sourceTime, legacy step` 258,
  all matched.
- The corpus case: `engine/corpus/src/projects/time.ts` gains `retiming/variable-speed`
  (eased speeds and an eased speed with an expression on videos and precomps at the
  root, an eased speed in a 24 fps composition nested in the 30 fps root, samples off
  the grid). `bun test` and `generate.ts --check` pass (172 projects, 0 invalid). Corpus
  v1 was captured before it, so the next capture records it; it was not captured here.

## T5 and T6: two writers that did not announce their writes

**Defects.** `PM.assets.restoreProject` hydrated `meta.format` on an older project's
asset record with no announcement (the auditor's missed write in
`import-svg.spec.ts:7`). The `requiresEngine` stamp is set and removed by a bus listener
after the event's own announcement, so routed queries answered from a mirror without it
(three stale answers in `media-store.spec.ts`).

**Fixes.** The gate's verified patches: `restoreProject` calls `PM.touch?.()` once when
it hydrated a format (optional call, as the stamp listener's other writers do, because
test registries have no `touch`); the stamp listener calls
`PM.doc?.touched?.([['requiresEngine']])` after `applyEngineStamp` changed the project.

**Evidence.**

- `legacy/gl/raster.test.ts` "announces the format it hydrates on an older asset
  record" and `media/host-media-store.test.ts` "announces each requiresEngine write to
  the write barrier" fail without the fixes and pass with them (both files 43 tests).
- `VITE_POWERMOVE_NATIVE_EVALUATOR=audit POWERMOVE_ENGINE_FONTS=1`, `import-svg.spec.ts`
  and `media-store.spec.ts`, `--repeat-each=2`, serially under the lock: 30 of 30 passed;
  46 launches; **937 audits, 0 failures** (0 missed writes, 0 mirror mismatches, 0 stale
  answers); 31,648 dual answers, 0 mismatched, 0 stale; 0 consistency mismatches, 0
  tracking violations. (The gate's control run without the fixes had 8 failures in 914
  audits.)

## D1: the last frames of video clips in exports

**Defect.** `pm-render` clamped every video source time to `dur − 0.04`, the padding
today's compositor uses for its own seeks (the viewer). Today's exports and agent stills
run `prepareFrame` first (`frame-preparation.ts`, `exporter.ts` `run` and
`snapshotAsync`), which seeks every clip with `videoPlaybackTime(asset, t, 1 /
PM.proj.fps)`, so they keep a clip's last frame. A 20-frame clip in a project of its
length lost its last frame at 30 fps and its last two at 60 fps in engine exports.

**Fix.** `RenderRequest::video_end` (`VideoEnd::Preview`, the default, 0.04 s;
`VideoEnd::Final`, `1 / fps` of the composition rendered). `VideoEnd::of(purpose)`
makes exports and agent captures final (`FramePurpose::is_final`); viewer, scrub, and
thumbnail requests are previews, as today's `renderToPixels` without `prepareFrame` is.
The job engine's export plan and agent stills (`pm-export/src/render.rs`) ask for
`Final`; the viewer's compose (`pm-node/src/present/compose.rs`) states `Preview`.
`plan::build` takes the kind and `playback_time(time, dur, padding)` applies it.
Precomps inside the exported composition keep its padding, as `prepareFrame` does.

**Evidence.**

- `pm-render/tests/video_end.rs`: a 20-frame 64 × 64 H.264 clip of 20 levels (the
  clip `export-video-frames.spec.ts` makes), rendered at every frame of a project of
  its length at 10, 24, 25, 30, and 60 fps and read back. `Final` shows all 20 source
  frames at every rate and equals today's rule (the padding, Chromium's microsecond
  seek, the last frame starting at or before it); `Preview` equals today's rule with
  0.04 s (the last frame hidden at 30 fps, the last two at 60). With the old padding the
  test fails at 30 fps: `[…, 17, 18, 18]` against `[…, 17, 18, 19]`. Unit tests check
  the purposes and `playback_time`.
- `cargo test --release -p pm-render --lib --test headless --test video_end`,
  `-p pm-export --lib --test job_export`: pass.
- `POWERMOVE_NEW_ENGINE=1`, `export-video-frames.spec.ts`: **9 of 9 pass**, including
  the three MP4 cases at 30 fps that failed at the gate.

## D2: a project installed by assigning `PM.proj`, and the Play start

**Defect 1.** Assigning `PM.proj = PM.mkProject(…)` resyncs the engine with the new
document (a `load` batch) but emits no project event, so the viewer kept sending the
old composition id in its viewer state. The presenter could not resolve that
composition in the new document: no frame was drawn (the session's frame count stayed
at the old project's 2), and Play produced a stopped clock (0 s in 1.5 s in a probe).
Opening a project (`pm-open-project`) emits `projects:open`, whose layout sends the new
composition.

**Fix 1.** `DocumentSession` gains `onLoaded`, called after the resync a `load` batch
causes (also when that resync fails, logged); the connection then emits the bus's
`layout`, so the viewer lays out for the new document as opening a project makes it do
(the new composition, its fit). `engine-connection/session.ts`, `install.ts`.

**Defect 2.** Play took about 300 ms to move the playhead on the Bluetooth output.
Measured in the app (probe of clock samples against the Play instant):

| | Run start after Play | Of which |
|---|---|---|
| First Play after launch | 327 ms | device open about 150 ms, output latency 160 ms, pre-roll |
| Later Plays | 176 to 181 ms | output latency 160 ms, pre-roll 16 to 21 ms |

Today's transport warms its AudioContext while idle when the project has audible
layers (`requestContextWarmup`), and its playhead starts at Play: it does not wait for
the device to open, and for a project with nothing to hear there is nothing to wait
for. The engine added both waits: it opened the device at the first Play, and every
run, silent ones included, waited for the device to present its first buffer.

**Fix 2.** (`pm-node/src/present/render.rs`, `audio.rs`; `pm-audio/src/graph.rs`)

- A composition with nothing to hear (`pm_audio::graph::audible`: the mix graph's walk
  over audio layers with media, videos with unmuted embedded audio, precomps, solo, and
  on states, without opening any media) plays on the host clock from Play, after the
  line's 50 ms frame pre-roll, counted in the presenter's new `silentRuns` (not in
  `hostClockRuns`, which still counts device failures).
- Each document view's audio thread starts with the view and opens the output device
  ahead of the first Play once the composition is audible and the controls have rested
  for 100 ms (`AudioCtl::Warm`, `WARM_IDLE`), as today's warm-up does on idle. The mixer
  still gets the project at Play, with the media the engine has resolved by then (so no
  transient "the media is missing" problem is recorded while an import resolves).
- Audible runs still start with the first buffer heard: the playhead holds for the
  output latency so frames stay in sync with the sound (section 15; the A/V sync bar).
  Today's playhead does not hold, and today's sound comes the output latency after its
  picture; that difference is kept by design.

| After the fix | Run start after Play | Playhead moving | Advanced in 1 s |
|---|---|---|---|
| Silent project (`PM.proj` assigned), 3 runs | 50.3 to 50.8 ms | 52 to 62 ms | 0.951 to 0.957 s |
| Audible project, first Play after import, then 2 more | 181 to 198 ms | 184 to 203 ms | 0.805 to 0.827 s |

**Evidence.**

- `engine-connection.test.ts` "asks the viewer for its state again once another
  document is loaded" (no call for edits or gaps; one after the `load` resync, with the
  new document id; none after close): fails without fix 1.
- `pm-audio` `graph::tests::audible_finds_the_clips_the_mix_would_play_without_media`.
- `pm-node/tests/document.rs` `a_silent_composition_plays_from_play_without_waiting_for_the_audio_device`
  (child-process engine with the audio device in use): the first playing sample's run
  start is at most 70 ms after Play, with no output latency, `silentRuns` 1,
  `hostClockRuns` 0, no device opened. Without fix 2 it fails: "the run starts 282276
  µs after Play".
- `POWERMOVE_NEW_ENGINE=1`: `export-mp4.spec.ts` **2 of 2 pass** (`:61` advanced 0.145 s
  with fix 1 alone, then passes with fix 2); `native-engine-tracer.spec.ts` 10 of 10,
  including both clip tests ("plays … with its audio": `hostClockRuns` 0, the device
  open, no audio problems) and "playback runs on the engine clock" (±10 ms A/V lines).
- Not changed: a host-clock run that becomes audible by an edit plays its sound from
  the next Play or seek. The paused, open device counts an underrun on every callback
  (`ring.rs`, an empty ring); that was already so after the first Play and is now so
  from the warm-up (451 in a probe of about 20 s); only the measurements read it.

## R1: image-sequence exports with the flag off

**Defect.** `import-image-sequence.spec.ts:7` (12 and 120 fps) failed intermittently with
"The GPU could not capture the export frame": `renderToPixels` returned no frame. At the
integration tip 7 of 12 runs failed (flag off). Every failure is the export after the
relaunch (`restored = await exportPixels(1 / fps)`, spec line 109).

**Cause.** The compositor's WebGL context is the viewer panel's: the viewer calls
`GL.init` with its canvas when it mounts. After the relaunch the spec opens the saved
project file and exports as soon as the asset is in the asset map. Instrumented runs
(the compositor's `GL.init`, `renderToPixels`, and `Export.run` timed from the open):

| Build | Export starts | First `renderToPixels` | `GL.init` | Result |
|---|---|---|---|---|
| `2b6ac8d3` render (wave 3) | 207 to 664 ms | 585 to 828 ms, context present | 137 to 582 ms | 8 of 8 pass |
| `9060f251` store (wave 3) | 196 to 1,265 ms | one at 233 ms with no context | 177 to 349 ms | 7 of 8 pass |
| `f47b1f8c` tip | 126 to 183 ms | 149 to 216 ms with no context (7 runs) | 177 to 284 ms | 1 of 8 pass |

So the export never waited for the viewer's context; it only used to finish its open
and frame preparation after the viewer had mounted. From the store lane's wave 3 merge
on (the gate's bisect) the open and the preparation got faster, and at the tip the
render nearly always comes first. The same race reaches any export or agent still asked
for right after a project opens, before the editor's viewer mounts.

**Fix.** `GL.whenReady(timeoutMs = 10 s)` in `legacy/gl/compositor.ts` resolves when a
context exists (at once if it does; when `GL.init` succeeds otherwise). `prepareFrame`
(`legacy/core/frame-preparation.ts`), which every export path and agent still runs
before its synchronous GPU compositing, awaits it when the compositor has no context. A
wait that times out (a workspace without a viewer) is not repeated per frame until a
context arrives, so a video export without one fails once with the same message rather
than waiting at every frame.

**Evidence.**

- `compositor.test.ts` "waits for the viewer to give it a canvas, and stops waiting per
  frame after a timeout" and `frame-preparation.test.ts` "waits for the compositor
  context before preparing a frame".
- With the fix, the instrumented runs show the export starting before `GL.init` (273
  against 341 ms, 252 against 314 ms) and rendering after it (607 to 650 ms).
- `import-image-sequence.spec.ts:7 --repeat-each=12`: **24 of 24 pass with
  `POWERMOVE_NEW_ENGINE=0`** (2.9 min) and **24 of 24 with `POWERMOVE_NEW_ENGINE=1`**
  (5.3 min).

## Files outside this lane's own

Minimal edits, listed for the owners: `pm-export/src/render.rs` (two requests ask for
`VideoEnd::Final`), `pm-node/src/present/compose.rs` (one field), `pm-node/src/present/render.rs`
and `audio.rs` (the silent run and the warm-up), `pm-audio/src/graph.rs` (`audible`, a walk
without media), `pm-eval/tests/support/layer_replay.rs` (the engine-step check),
`legacy/gl/compositor.ts` (`GL.whenReady`), `engine/corpus/src/projects/time.ts` (one
project), `docs/engine/pm-eval-layers.md` and `docs/engine/tracer.md` (the notes these
changes made wrong).

## Reproducing

```sh
source engine/env.sh
cd engine
cargo test --release -p pm-eval --test layer_parity --test layers_unit
cargo test --release -p pm-render --test video_end
cargo test --release -p pm-node --test document
cargo test --release -p pm-audio --lib graph
cargo build --release -p pm-cli && target/release/pm-cli parity eval
cd ../apps/desktop
bunx vitest run src/renderer/src/engine-connection src/renderer/src/media/host-media-store.test.ts \
  src/renderer/src/legacy/gl src/renderer/src/legacy/core/frame-preparation.test.ts
node scripts/build-engine-addon.mjs
L=~/Library/Caches/powermove-engine/bin/pm-electron-lock
VITE_POWERMOVE_NATIVE_EVALUATOR=audit POWERMOVE_ENGINE_FONTS=1 POWERMOVE_NATIVE_REPORT_FILE=audit.jsonl \
  $L bunx playwright test e2e/import-svg.spec.ts e2e/media-store.spec.ts --repeat-each=2 --workers=1
node scripts/native-evaluator-report.mjs audit.jsonl
POWERMOVE_NEW_ENGINE=1 $L bunx playwright test e2e/export-video-frames.spec.ts e2e/export-mp4.spec.ts \
  e2e/native-engine-tracer.spec.ts --workers=1
for f in 0 1; do POWERMOVE_NEW_ENGINE=$f $L bunx playwright test e2e/import-image-sequence.spec.ts:7 \
  --repeat-each=12 --workers=1 --retries=0; done
```

## Rerun correctness fixes: D4, D5, R3, R2 and the two test writers

Verified 2026-10-18 on `feat/ne-races`, integration base `9f6e8f27`, the same
M4 Max reference machine. Release addons and pm-wasm built in the worktree;
hidden Electron under the machine lock, one worker, no retries.

**D4 and D5 share the import-binding race.** The renderer exposed a completed
import before the debounced asset binding reached main. An immediate export's
offline check saw an unknown asset; the interactive engine could also resolve
newly imported images before main knew them. The integration branch already
contains the fix, `6a2f9587`: `ingestAsset` awaits
`HostMedia.flushBindings(targetProject)` before returning, including silent,
parallel imports. This lane keeps that fix and adds a multi-import regression.

The negative control removed **only** that await and rebuilt the app. All three
reproductions failed: `export-mp4.spec.ts:7` returned `{cancelled: true}`;
both generated PNGs in the new readiness test resolved to null immediately;
the synthetic 500-layer measurement recorded 41 layer failures. Restoring the
await makes all three pass, with zero render/layer failures in the synthetic
project (two imported images used by 35 image layers).

`media-import-readiness.spec.ts` runs with the flag off and on. It checks both
images are imported, resolves each through main immediately with no polling or
extra flush (flag on), exports immediately with no offline prompt, and checks
both exported regions are exactly opaque red and green. Assigning the regions
does not assume a parallel import's completion order.

**R3 was lazy temporal-ease adoption.** Hydration sanitized ordinary channels
but left `powermove.3d.scene` animation inside extension data untouched. Legacy
evaluation then added defaults that native evaluation did not write. Which
frames had evaluated determined whether a flag-off open changed the document.
`legacy/app.ts` now adopts and normalizes these scene channels during hydration,
including nested compositions. Bare scene keys retain their linear behavior:
adoption precedes the general sanitizer's ordinary layer default handles.
The regression checks object, material, light, camera and environment tracks,
quarter/midpoint values, unchanged JSON after evaluation, and idempotent reopen.
It fails before the fix and passes after it. `scene3d.md` records the change.

**R2 and test writers.** The alpha ProRes preset expects canonical
`prores-4444`, still supplying old `prores` to test migration. The render-queue
rename and tracer work-area assignment call `PM.touch()`. The affected import
rerun also reached three old assertions that predate preservation of originals.
Native TIFF/HEIC/Matroska imports now verify source bytes by BLAKE3 and separate
PNG/WebM renditions; legacy mode still verifies stored conversions. Existing
pixel, alpha, frame-order and duration assertions remain unchanged.

**Verification:**

- Four targeted unit files (`legacy/app`, renderer `host-media-store`, main
  `media-store/service`, `engine/offline-media`): **78 pass**.
- All 11 affected cases (`export-mp4`, `export-webm`, `import-animated-image`,
  `import-converted-formats`, the saved-preset case) pass with the flag off and
  on. The first native run passed all seven D4 reproductions, then failed the
  three obsolete storage assertions; those pass after the spec update. The
  new two-mode image readiness/pixel test also passes.
- The two writer reproductions and saved-preset case in `audit`, with engine
  fonts: **3 pass; 76 audits, zero failures**, including zero missed writes,
  mirror mismatches and stale answers; 777 dual answers, zero mismatched/stale;
  zero consistency mismatches and tracking violations.
- Unchanged engine-switch corpus harness: **131/131 in both runs**, 71 s and
  67 s, including `3d/legacy-scene`. Each compares 131 baseline, control, off,
  and on-again documents; 65 assets across 60 media projects; zero failed or
  skipped projects and zero page errors. Its existing on→on control still
  records regenerated key ids in 16 effect projects; switching adds nothing
  beyond that. This is inherited harness behavior, not a new exception or a
  claim that those 16 raw document hashes are stable.

No main/engine, preload, kernel API or raster changes were needed in this lane;
the temporary negative-control raster edit was restored exactly. No capability
reason, routing path, tolerance or corpus case changed. Both requested
integration merges were already up to date.

Reproduce with the existing commands above plus
`e2e/media-import-readiness.spec.ts`; the synthetic case is
`PM_TRACER_MEASURE=1` with `native-engine-tracer-measure.spec.ts --grep 'synthetic 500-layer'`.
The auditor cases are `engine-delivery.spec.ts:199`,
`native-engine-tracer.spec.ts:348`, and `ae-audit-implementation.spec.ts:102`, with
`VITE_POWERMOVE_NATIVE_EVALUATOR=audit POWERMOVE_ENGINE_FONTS=1` and
`POWERMOVE_NATIVE_REPORT_FILE` outside the repository. Run every e2e under the
lock with `--workers=1 --retries=0`. Run
`scripts/gate-engine-switch-corpus.mjs --json <outside-repo-path>` twice under
the lock for the corpus gate. Build both `build:engine` and `build:wasm` first.


## K1, B1 and R3b: stable keys and export bindings

Follow-up on `feat/ne-fixups`, integration base `01a9f4af` (2026-10-18).
Both requested integration merges were already up to date. No router,
capability reason, corpus case, tolerance or allowed-difference rule changed.

**K1.** Known effect parameters are sanitized twice against one global key-id
set: a loose pass (keys typed by the saved value, or 0) and then the
definition's pass (value typed by `param.def`). The second pass saw the
channel's own ids as duplicates and replaced them. Each registered channel now
releases its own ids from the set just before its second pass; both passes and
their value and keyframe semantics are unchanged. (A first version of this fix
merged the passes into one def-typed pass. Review showed that changed today's
hydration of null, missing and mismatched-type values, which owner rule 22
forbids, and it was replaced.)

Two regressions: the id test preserves numeric, color, boolean and
unknown-parameter ids and requires byte-stable JSON on reopen; it fails against
the base (registered ids replaced). The parity test hydrates `{v:null}`, a
primitive `null`, color and boolean keys with no `v`, a numeric param with
`v:'5'` and keys `[1,5]`, and a color param with `v:3`, and pins today's
results: value 0, 0, def, def, def, def, and no keyframes for all of them. It
passes on the base app.ts except for the key-id assertion, and fails against
the single-pass version (values became def and the dropped keys animated).

**B1.** Export and Add to Queue await the media binding acknowledgement, as does
the agent export's `__mcp_context`. They retain their captured project while
awaiting it. IndexedDB-only media finishes verified migration before the barrier
answers. Already imported media binds immediately from cached records: adding an
unconditional migration await here reproduced two import-time offline layer
failures, which the unchanged zero-layer-failure assertion caught. That await
was removed for cached media, and the original assertion passes again.

Queue entries persist copied-media hashes at Add to Queue. Every export snapshot
binds copied hashes and linked originals to its job session before resync; host
capability resolution now carries engine process and session. Project switches,
window release and later writes under the same storage key cannot replace those
bindings. Ending the job releases them. Existing live-window ambiguity checks
and capability-port validation remain in place.

A job's bindings are not a dead end. Each copied asset keeps its storage key
next to the frozen hash (also when no record existed at snapshot time), and the
job remembers its project id. Resolution tries the frozen entry first (link,
then hash object, then key record, the same order as the window path). When that
is offline it falls back to windows bound to the same project id, with the usual
ambiguity check; windows on other projects never count. So `wait_for_media`'s
2 s re-mint resumes a queued or sheet job when copied media that was offline at
enqueue is put later, when the user relinks with Locate Media in the project's
window, or when a saved hash object is missing but the key's record exists. The
regression covers all three and that another project's window is ignored; it
fails against the first B1 version.

Negative controls fail without the sheet/agent acknowledgement and without job
binding. The unit tests cover migration, immutable queue content, copied and
linked media after window release, process/session isolation and session cleanup.
The expanded image-readiness e2e retains immediate post-import resolution and
zero import-time layer failures, then exports after reopen with no live binding,
and renders a queued snapshot after switching projects and releasing the
window's bindings. Every exported red/green sample must remain exact opaque
RGBA, with no offline prompt.

**R3b.** The legacy-scene regression now has color, boolean and id-less keys in
root and nested compositions. It compares legacy answers at quarter/midpoint,
retains the numeric 22.5/45 assertions, checks ids, and requires unchanged JSON
after evaluation and reopen. Removing scene-channel adoption fails the test.

`media-import-readiness.spec.ts`'s native presentation check requires an **awake
display and unlocked session**, as the presentation harness does. Keep that
assertion unconditional when running this test: do not interpret a sleeping
compositor's lack of presentation as an import or export failure. The media,
export pixels and layer-failure assertions remain separate requirements.

Outside the lane's named files, the changes are: main's engine host callback
types and wiring (`main/engine/{index,native-engine,services}.ts`); the
`bindMediaSession` hook in `main/engine/agent-jobs.ts` and its test; the agent
snapshot call site and test (`legacy/assistant/mcp-project-tools`); the
`e2e/media-import-readiness.spec.ts` extension (reopen export, queue after
project switch); and notes in `export.md`, `media-store.md` and `scene3d.md`.
The hot `main/index.ts` edit is only the three media callbacks. No preload,
kernel API, editing or raster implementation changed.

Remaining: `flushBindings` migrates uncached IndexedDB media before a sheet export
or queue add. For large media that delays the start by a full copy, and the
progress sheet shows nothing while it runs. It is correct but has no UI
feedback yet.

**Verification:**

- Release `pm-node`/`pm-bridge` and `pm-wasm` build successfully.
- Nine targeted unit files: **129 pass** (hydration, exporter, agent context,
  renderer/main media store, agent jobs, host services/contract and render queue).
- Image readiness, reopen export and queued snapshot pixels: **2 pass**, flag
  off/on. The original presentation and zero-layer-failure assertions pass.
- MP4 delivery with AAC and export cancellation: **2 pass with the flag off**,
  **2 pass with it on**. Hidden Electron, lock held, one worker, no retries.
- Unchanged engine-switch corpus: **131/131 twice**, **70 s and 67 s**; each
  compares 131 baseline/control/off/on-again documents, 65 assets in 60 media
  projects. Zero failures, skips, page errors **and unstable reopen documents**.
  The earlier control's 16 effect projects no longer regenerate key ids.
- Extension boundary check passes. Main TypeScript checking passes with
  `--composite false --incremental false`; the standard composite command is
  blocked by existing TS6307 file-list errors from `shared/canonical-json.test.ts`
  importing renderer `compositions.ts` and `registry.ts`. Those files are unchanged.
- Every commit's staged file list/diff and gitleaks scan were checked; the local
  hook remains enabled. No push, integration-branch merge, tag or release.

**Verification after the review corrections** (K1 two-stage restore, B1 job
fallback), app rebuilt by the e2e global setup:

- Eleven targeted unit files (the nine above plus `media-store/store` and
  `engine/offline-media`): **148 pass**. Negative controls as described under
  K1 and B1.
- MP4 delivery and cancellation: **2 pass flag off, 2 pass flag on** (lock,
  one worker, no retries).
- Unchanged engine-switch corpus: **131/131 twice, 69 s and 73 s**, zero
  failures, skips or stamps.
- `media-import-readiness.spec.ts`: flag off passes; flag on fails at the
  presentation poll (line 66) because the machine had been idle about 11 h with
  the display asleep, the precondition stated above. A temporary untracked copy
  that skipped only that poll (and the presenter check, since no presenter
  exists while asleep) passed **2/2**, covering import readiness, export pixels,
  reopen export and the queued snapshot after a project switch. The copy was
  deleted. The committed spec is unchanged and still needs an awake display.
- Web and node `tsc --noEmit --composite false`: no errors in changed files.
  The two R3b test lines that failed `noUncheckedIndexedAccess` were fixed.

Reproduce from `apps/desktop` after sourcing `../../engine/env.sh` and building
`build:engine` and `build:wasm`. Run `bunx vitest run` on the nine files above with
`--maxWorkers=4`; run `e2e/media-import-readiness.spec.ts` and
`e2e/export-mp4.spec.ts` under `pm-electron-lock` with `--workers=1 --retries=0`,
setting `POWERMOVE_NEW_ENGINE=0` and `1` for the MP4 spec. Run the unchanged
`scripts/gate-engine-switch-corpus.mjs --json <outside-repo-path>` twice under
the lock. The readiness spec supplies both engine settings itself.
