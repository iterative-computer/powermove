# P2 extension checks

**OPEN / NOT PASSED.** Trace-only comparisons of four-step scripts (reload, two
seeks, dispose) against Electron at `c7f46a05`. No pixels, no panels, no complete
interaction scenarios: a match here is not an accepted extension.

## 2026-11-05 — first honest comparison of all 58

`tools/extcheck/trace_compare.py`, three recordings per role, native logic in the
real compat realm (rusty_v8, production kernel adapter and transport).

| Population | Total | Match (ignoring key order and ids) | Mismatch | Can't run |
| --- | --- | --- | --- | --- |
| Public fixtures | 10 | 8 | 1 | 1 |
| Owner corpus | 44 | 28 | 8 | 8 |
| Public Store | 4 | 0 | 0 | 4 |
| **All** | **58** | **36** | **9** | **13** |

No case matches byte for byte: the kernel keeps JSON keys sorted, so every row
differs in key order, and `exact` comparison stays a mismatch.

Mismatch families, by cases:

1. **9 — panel retirement.** All nine are the same 12 rows. When a panel's last
   definition goes, Electron hides it by forking the kernel workspace; the native
   app hides the tab in the shell's layout and leaves the kernel workspace alone,
   so `workspace.mutate` and its events never appear. It stays a mismatch.

Can't-run reasons, by cases:

- 5 — owner inputs absent from the frozen corpus (39 of 44 present).
- 3 — Store: facade calls inside a confined helper aren't recorded. All four public
  releases do activate natively there.
- 3 — the reference itself never activated the input (a missing source file, a
  dependency outside the one-extension profile, a fixture needing Store permissions).
- 1 — native activation: reads the built-in `viewer` service, which doesn't exist.
- 1 — Store: the reference's own three recordings differed in this run.

This run covers lifecycle and seeking only. Most of what real extensions do in
panels is not yet scripted, so rendering, DOM and interaction gaps do not show here;
see [extensions-app.md](extensions-app.md).

---

Earlier rounds, kept for the blockers they recorded:

**OPEN / NOT PASSED.** No extension has accepted paired Electron/native traces,
visual oracles and complete scripted-interaction coverage. Successful activation
and component tests are not extension matches.

Reference: frozen `v1.2.0`, `07ed5fa17ee640659cb075f9cad1af2095011d60`.
The earlier check's executable and harness were brought forward from its branch;
they were not present in the fetched native-engine branch at continuation time.
The S1 sandbox archive and bindings passed all five checksum checks. Builds use
Rust 1.96.1, `engine/env.sh`, the shared target and the build lock. No release,
app update, tag or deployment was created.

## 2026-11-01 — round-3 WIP handoff

The branch fast-forwarded to `origin/feat/native-engine` at `3c4fbc4e5`.
The results below remain the last completed all-source run; **no new accepted
reference or paired native recordings were produced this round**. The queued
three-recording probe never acquired the shared GUI lease and has been stopped.
No app was launched by this round's probe; the other lane's lease was untouched.
All pending lane-owned build/probe controllers were stopped for handoff.

Run from the checkout root. Reuse the prepared frozen reference; do not blindly
reinstall its trimmed dependencies. Inputs/profile remain read-only. The current
floor is **5 GB decimal**, measured by `diskutil`, superseding the earlier floor.
The wrappers source the engine environment, use the build/GUI leases and monitor
pause/active flags. Never remove another lane's lock or either flag.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4
export PM_S1_ARTIFACT=/private/tmp/pm-s1-v8-artifact-final
export ELECTRON_OVERRIDE_DIST_PATH=/private/tmp/pm-anchor/node_modules/.bun/electron@44.0.0+759ce506b1ed1a42/node_modules/electron/dist
export TZ=UTC
diskutil info / | grep 'Container Free Space'
# Headless checks/builds: nice -n 20 and flock /tmp/pm-build.lock are inside build.sh.
bash tools/extcheck/build.sh test
bash tools/extcheck/build.sh clippy
bash tools/extcheck/build.sh fmt
bash tools/extcheck/build.sh build
nice -n 20 node tools/extcheck/test_clock_fixture.mjs
nice -n 20 python3 -m unittest discover -s tools/extcheck -p 'test_*.py'
# GUI commands acquire gui.lock themselves. Run only with both flags absent.
test ! -e /tmp/pm-shots/PAUSE_LOCAL && test ! -e /tmp/pm-shots/USER_ACTIVE
nice -n 20 python3 tools/extcheck/repeat.py --reference-root /tmp/pm-extcheck/reference --store --repeats 3 --output /tmp/pm-extcheck/round3-reference-stability.json
nice -n 20 bash tools/extcheck/run.sh --reference-root /tmp/pm-extcheck/reference --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --store --repeat 3 --summary /tmp/pm-extcheck/round3-summary.json
```

Findings/normalization: the earlier investigated pair varied in a generated
workspace ID, the timeline's clock-driven spring trail and the live frame-ms
readout. The existing workspace-ID alias makes that pair's traces equal without
changing fixture state. Its pixels still vary. The new `clock-fixture.mjs`
provides explicit frame/time progression and seeded entropy; **it is tested in
isolation but is not installed in either capture driver yet**. No new trace
field or pixel is ignored. No per-extension volatility allowlist exists yet.
Window/device scale, fonts, asynchronous activation and clock integration still
need repeated measurements and justified policies. `repeat.py` now collects at
least three captures per case; `stability.py` records differing paths and pixel
bounds in private output only. These new CLI paths have not had a GUI run.

Completed this round: clock-fixture unit probe and 24 Python tests pass; shell
syntax and Rust formatting were checked. The native transport regression first
reproduced `PM.setTime` missing (20 other runner tests passed). A WIP native
worker transport and JS facade were then added, including seek/frame rounding,
play/pause/loop and explicit refusal of unattached media playback. **Post-fix
Rust tests, build and Clippy remain pending**: the last check was paused below
5 GB and cancelled for handoff. The transport is not evidence of completed
native capture or production media transport.

Remaining: install/verify the same deterministic fixture in both hosts; inspect
three or more recordings for every case and maintain a reasoned per-extension
allowlist only where genuinely needed; complete the native script/trace/PNG
endpoint and recorder-to-DOM presentation; finish Store release admission and
resource/view/callback dispatch without fabricating signatures/grants or running
public Store code trusted; use the supplied projects to author/review full
per-extension scenarios; then rerun all 58, rank measured mismatch families,
fix small repros and update the table. The bundled native driver still stops at
activation/disposal, so the final command above currently reports P2 OPEN.

## Population and current evidence

| Source | Total | Match | Mismatch | Can't run | Logic executed in real V8 | Activated |
|---|---:|---:|---:|---:|---:|---:|
| Owner installations | 44 | 0 | 0 | 44 | 44 | 44 |
| Local public corpus | 10 | 0 | 0 | 10 | 9 | 9 |
| Published Store, latest releases | 4 | 0 | 0 | 4 | 0 | 0 |
| **Total** | **58** | **0** | **0** | **58** | **53** | **53** |

Activation counts are from the rebuilt runner's complete one-case-at-a-time
headless V8 audit, after the kernel and compatibility fixes below.
The previous baseline activated 36 of the same 53 executable inputs. Populations
remain separate, without cross-source deduplication or unsupported-extension
exclusions. Owner discovery counts immediate manifest-bearing installations:
24 v1 and 20 v3. One local fixture declares permissions with apiVersion 1 and
is rejected by the frozen manifest parser; its rejection remains unchanged.
Store archives are compiled but never substituted into the trusted owner realm.
The refreshed anonymous Store catalog still has four latest releases. All four
compile and enter the isolated Store facade on the frozen Electron reference.
None executes in the native helper: native release admission remains unavailable.
The catalog was re-read after the all-source run without collection drift.

The all-source harness completed fresh attempts for every one of the 58 cases,
with both roles attempted independently and each copied case deleted afterwards.
These are comparison **attempts**, not 58 complete paired captures:

| Source | Reference repeat failures | Reference not active | Native full-capture blockers |
|---|---:|---:|---|
| Owner installations | 43 | 1 | 44 missing native script/trace/PNG endpoint |
| Local public corpus | 9 | 1 | 9 missing endpoint; 1 invalid frozen manifest |
| Published Store | 4 | 0 | 4 missing native release admission |

No role supplied an accepted repeatable full capture. The native driver still
stops after activation/disposal. Its missing WebKit/script endpoint is explicit;
the standalone WebKit probe does not satisfy it. Store resource dispatch and
complete per-extension reviewed scripts/projects are also unfinished. The
incomplete default smoke script was used for this attempt and cannot prove P2.
No source was excluded from the denominator.

## Activation investigation

All 17 original failures were reproduced before changes, using copied inputs
and the actual production bundle inside V8. Their immediate causes were:

| Family | Original failures | Immediate cause |
|---|---:|---|
| DOM panel | 11 | Missing panel open/close/isOpen backend |
| PM facade | 5 | Missing `PM.invalidate` |
| Lifecycle / DOM scheduling | 1 | Missing host animation-frame scheduler |

The fresh headless audit now removes all 17 original activation failures:
all 53 executable owner/corpus inputs activate in the real V8 runtime.
Deeper failures exposed an absent icon helper/DOM string conversion and an
incorrect initial project/composition representation, which were fixed in the
previous continuation. The last blocker, shared controls, is now connected to
the frozen Svelte document, selection and transport stores. Seven unchanged
controls are compiled from the pinned reference: SelectField, TextField,
ToggleField, Row, Section, Segmented and Disclosure. The other declared controls
still fail explicitly when requested; activation does not exercise every API.
The production bundle/CLI regression mounts TextField and SelectField, publishes
a native history edit, changes the select through its real event handler, and
undoes that edit while checking the displayed input value.

The runner preserves the underlying reported error rather than only returning
the generic activation-error wrapper. No owner source, names, identifiers,
selectors or error strings enter tracked files.

No **measured cross-host mismatch** ranking exists yet. The table above is an
activation-blocker classification, not an Electron/native mismatch ranking.

## Adapter implementation and limits

- **Kernel:** the runner owns a disposable `pm-kernel::Kernel`, without a profile
  or journal directory. Calls use the native history/transaction authority;
  replies carry authoritative document, selection, workspace and history facts.
  The adapter handles direct writes, history begin/commit/cancel/rollback,
  undo/redo, external closures and cleanup pins, command batches through the
  existing native command planner, selection and workspace writes, plus the
  synchronous engine-query boundary. Immutable request/reply caching preserves
  retries and rejects transaction-ID reuse with different requests. Unsupported
  methods return acknowledged errors. Structural callback leases, complete PM
  facade methods, built-in provider closure and rendering backends remain open.
  Publications and history snapshots expose the legacy open-composition shape;
  direct-write operations are staged in that shape and translated back to the
  native canonical document before the authority applies them. The synthetic
  initial project now comes from the same complete factory as the reference.
- **Panels:** `CompatPanels` invokes actual contribution builds in the V8 DOM
  mirror, tracks open/hidden state, refreshes registry overrides and disposes
  owned roots/imperative cleanup functions. `PM.invalidate` flushes the DOM queue
  and marks presentation dirty; animation frames use runtime-owned timers.
  Opening a mirror contribution is not a WebKit paint acknowledgement.
  The icon helper is extracted unchanged from the frozen public source. Shared
  Svelte mounting/reactivity modules are loaded for the host even when extension
  source imports neither; optional browser modules remain demand-loaded.
- **WebKit:** `MacDomHost::new_remote_dom` accepts the host-built production
  `PageApplier` bundle, using a dedicated document per realm. `RemoteDomPanel`
  validates mutation batches through `pm-remote-dom::Router` on a routing worker
  before queueing admitted batches to AppKit. The UI only presents admitted
  batches and retains the front batch on backpressure. Actual page feedback
  enters a bounded routing mailbox; its default handler explicitly refuses an
  unattached mailbox. The router then forwards validated feedback to a bounded
  realm-owner queue. Queue failure does not advance sequence state; only page
  feedback advances the applied sequence. Detached pages may acknowledge/layout
  but cannot forward input.
  The CLI still does not create this host or run a complete capture script.
- **Store:** `KernelStoreAdapter` connects installed-release CAS and profile
  data-store state to the existing durable native kernel writer. It preserves
  other source metadata and refuses stale revisions, changed profile identity
  and collisions with installed local extensions. `StoreRuntimeHost` runs the
  unchanged frozen asynchronous Store facade in the real confined Store helper,
  with no PM global or trusted Compat facade. Each IPC call rechecks native
  installation, revision and live permissions. Core project snapshots and
  extension-private storage use the real kernel; report count/bytes and storage
  are bounded. Raw engine operations and staged writes are rejected. Disposal
  runs before helper replacement; the test verifies private storage survives it.
  Registration/view/callback resources still require a concrete native
  `StoreDispatch`; network transport is explicitly unavailable. The JSON bridge
  does not claim full structured-clone/type parity. This is a tested boundary
  and core connection, **not a complete public Store runtime**.
  The checked public release metadata supplies archive/tree hashes but no native
  signed-release envelope. All four current permission arrays are empty; that
  does not supply installer provenance. No signature or permission decision is
  fabricated, and no public archive runs through a trusted fallback.
  The disposable Electron profile now uses the shipping Store marker and
  verifies that the loaded record has Store isolation. No account, provenance
  database, grant or owner profile state is copied.

The recorder still supports its documented native subset: basic state,
paths/rectangles, transforms, pinned drawImage sources and pixel readback.
Complete DOM-canvas integration, typed ImageData, text/paint/encoding and engine
rendering remain open. No Canvas or visual tolerance exception was introduced.

## Small synthetic reproductions

`element.innerHTML = 42` reproduced a parser TypeError in the V8 DOM mirror,
while the independent DOM produced `"42"`. The setter now performs DOMString
conversion before parsing, including null-to-empty and Symbol rejection. The
regression covers ordinary elements, templates and SVG. The earlier explicit
browser-bundle export and comment-Range UTF-16 fixes remain in place.

The Store core now applies the frozen broker's storage tuple arity, 128 UTF-16
code-unit key limit and 256 KiB encoded private-storage limit. Synthetic native
regressions cover both sides of the key boundary, surrogate pairs, oversized
values, empty keys and extra arguments. The real confined-helper regression
also distinguishes missing values from null and verifies disposal runs once.
This is component/source-contract evidence, not a measured extension mismatch.

Passing native tests cover stored/canonical composition translation, native
history round trips and external closures, transaction retry identity, selection
refusal, presentation demand, bounded DOM admissions/feedback and Store session
permission revocation. The production page-shell/applier event round trip also
passes in the headless DOM harness. After GUI availability returned, the real
WebKit smoke example ran through the shared lease with the production
PageApplier and reported `remote_dom_webkit_applied=1`, then closed its window.
That acknowledgement remains component evidence only.

## Harness and resource handling

The shared GUI lease now waits and retries every 60 seconds instead of skipping
a busy lane. Waiting occurs outside the capture execution timeout. Screen state
is checked after acquisition; the lock file is never removed. A free slot was
obtained after waiting. The disposable frozen reference was instrumented and
built; an existing matching Electron 44 binary avoids a second installation.

Repeat-reference investigation found a generated workspace ID in behavioral
trace rows. A fixture-only identity alias now makes the investigated pair's
traces equal, without changing workspace state or authored strings; a synthetic
regression also checks that ID accessors are not invoked. The pair still differs
visually in the live timeline playhead glow and render-time readout. No masking,
tolerance change, replacement renderer or timing-value rewrite was introduced.
The complete all-source attempt records 56 nondeterministic reference pairs and
two reference activation failures. This is unavailable oracle evidence, not a
measured Electron/native mismatch family ranking.

Disk pressure is never counted as an extension result: builds/captures pause
below 6.5 GiB and resume at or above 6.5 GiB. Inputs remain one case at a time and
are deleted after use. Other lanes' targets and input profiles are untouched.

Build and capture entry points also honor `/tmp/pm-shots/PAUSE_LOCAL`, checking
every five minutes. A new local pause stops the owned process group, releases the
GUI lease, and retries the interrupted attempt after the pause ends. No lock or
pause file is removed. Both capture roles also wait while
`/tmp/pm-shots/USER_ACTIVE` exists, and stop their owned capture if it appears.
Builds/tests use `nice -n 20`, four Cargo build jobs and four Rust test threads.
GUI probes and the fresh all-source attempts ran only after availability returned.

Latest completed checks: compatibility 177 pass and typecheck pass; capture
harness 21 pass; native V8 runner 20 pass, including the actual confined Store
helper and durable kernel CAS; trace recorder 9 pass (1518 assertions); disposable
reference-profile regression pass. The expanded build-tool-to-V8 CLI regression
passes with real shared controls, native history edits and select/undo handling.
The prior unchanged remote-DOM suite has 39 passing tests and its typecheck;
WebKit headless contracts 25 pass and Store contracts 90 pass. All-target
Clippy (`--no-deps -- -D warnings`) and Rust formatting pass for the runner,
including the new Store modules. The updated wrapper also completes successfully.

Only anonymised aggregates and original synthetic probes belong in Git. Raw
sources, reports, traces, screenshots and selectors stay under disposable paths;
the installed profile is read-only. The final gate still requires completed
repeatable captures for every source, complete interaction coverage, and fixes
or explicit `can't run` evidence for every remaining case.

## 2026-11-04 — cloud continuation, shared fixture and portable checks

**P2 remains OPEN. No new accepted cross-host match.** The cloud lane starts
from `9253bc087` (`feat/na-extcheck2`) and merges `ed5482622`
(`feat/native-engine`); the final refresh also merges `ce36cb6fc` and `711f44ba0`. Linux cannot
establish Electron/macOS/WebKit behavior.
The owner installations and their reviewed scenario/project inputs are absent
from this container. No private names, source bytes or identifiers are tracked.

The native worker transport now builds and its seek/frame/play/pause/loop
regressions execute. A rejected seek with an invalid running-clock argument
previously changed `time` before returning an error; command admission now
publishes only a fully validated candidate. This preserves the prior playback
state on refusal (**N3**, correctness of authored state). No media-backend
capability is invented; playback with unattached media still fails explicitly.

Both capture preparations now use **identical source bytes**, seed and a SHA-256
fixture identity. `prepare-reference.mjs` inserts the fixture before the frozen
renderer module and adapts only its disposable generated startup helper. The
native CLI evaluates it before the compat bundle/extension. It covers Date,
performance, Math.random, UUIDs, integer-array entropy, timers and explicit
animation frames. V8's frozen performance object is handled without modifying
production runtime snapshots. Native remote-DOM scheduling retains the installed
fixture; timer/frame callbacks still enter the realm task boundary. The
reference driver rejects a stale/missing fixture, exposes boot-clock state as a
behavioral row, and supports explicit `advance` steps and actual host-paint
checkpoints. Neither boot state nor behavioral values/pixels are reset, masked
or normalized. **N3/N6:** control the test input; preserve observed differences.
Node and Bun's `Function.toString()` differ: hashing the original fixture source,
rather than function serialization, prevents silently installing different code.

The runner exposes optional DOM-batch inspection for the headless probe; queue
admission is still not WebKit paint. The new `probe.py` makes three independent
native recordings per executable public case and compares their complete JSON,
including activation, document/transport/clock, reported errors, disposal and
DOM batches. It does not declare a full trace/PNG capture or a P2 match.

The comparison runner requires at least **three** recordings per role and
collects all three before classifying instability. Repeat diagnostics now also
inspect interactions and use the gate's exact visible premultiplied-RGBA rule.
Population accounting preserves **44 + 10 + 4 = 58** even when input discovery
or catalog reads fail; it records expected, observed and unavailable counts.
Additional discovered cases remain visible and population drift prevents gate
acceptance. **N4/N6:** missing inputs and adapters never shrink compatibility
scope. Full captures on Linux return `macos-capture-unavailable` before touching
a GUI lease. macOS-only dependencies/probes are gated; portable host contracts
run on Linux. **N7/N8:** inspect workers headlessly and retain the main-thread
rendering boundary. No visual design was introduced; no new DESIGN-TODO(opus).

The native script/behavior-trace/PNG endpoint, recorder-to-DOM presentation,
Store installer provenance/resource/view/callback dispatch, and human-reviewed
per-extension projects/scenarios **remain unfinished**. Public archive hashes
are not the signed-release envelope required by the native installer, and no
preserved installation provenance is present here. No Store code is run with
the trusted Compat facade (**N4/N6/N8**). Completing those adapters requires
further implementation; a Mac rerun alone cannot close them. Exact continuation
commands and final verification counts are in
[handoff/extcheck-p2.md](handoff/extcheck-p2.md).


The final integration refresh records the owner's move of overall parity to
latest `main` (`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`). This report and its
frozen driver remain **v1.2.0 legacy-input evidence**, still required by that
decision and N4. They do not cover the newly required main deltas or establish
current-target parity. See the handoff for this additional open boundary.

## 2026-11-04 — owner reference change applied

The active comparison/compiler reference is now **main at
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`**. This supersedes the previous
continuation's legacy-only tool limitation. `tools/extcheck/references.json`
is the shared immutable pin registry. All capture requests and repeat reports
carry the selected revision; compiled bundles include `compiler.json`, and the
native driver refuses a mismatched compiler checkout before activation.
Preparation defaults to the current pin and refuses an old checkout unless its
legacy pin is explicitly supplied. Legacy gate success has a distinct
`LEGACY_PASS` label; it cannot become current-main `PASS` (**N3/N4/N6**).

A separate current-main reference checkout was prepared and built on Linux.
The build-tool-to-real-V8 controls/history regression passes against **both**
pins. The public corpus produced **9 repeatable native component probes per
pin**, with **three identical recordings per executable case**: **27 current
+ 27 legacy recordings**. The tenth case remains an invalid frozen/current
manifest in both pairs. No owner input, scenario or public Store code was
substituted with a synthetic success. Current-main full comparison still reports
**0 match / 0 measured mismatch / 58 can't run**, with four public releases
integrity-checked and no catalog drift. These are not WebKit or PNG acceptances.

Extension-area main-delta reconciliation (read with the complete source audit):

| Row / concern | Pinned source finding and disposition |
| --- | --- |
| A08 theme ownership, appearance and fork rebase | `theme-state.ts`, `workspace-appearance.ts`, `rebase.ts`, `update-fork.ts` are byte-identical between both pins. Required carry behavior remains in scope. Frozen helpers/component tests exist; full native theme lifecycle/rebase coordinator and paired scenarios remain open. |
| A09 extension publication and panel generation | `external-session.ts` and renderer `panel-tools.ts` are byte-identical. The finishing/panel generation fences already exist at the tag; the main bridge's surviving diff is resumable export-job work. Do not claim a newly ported fence. Native router/session generation tests are component evidence; application publication/recovery/capture-input fencing remains unverified and needs app/MCP wiring outside this lane. |
| X ledger `25bc8219c`, `37e9e0b71`, `c5fd263b1`, `e5bbfb677`, `908c89091` | Cleanup deadlines, retired-owner registration/shortcut rules, disconnected publication and serialized recovery remain required baseline carries. The host factory's surviving source diff is effects API exposure, not retirement semantics. Existing 177 compat tests and 20 Linux host tests do not establish all extension scenarios; retain the open lifecycle gate. |
| Compiler and initial factory | `main/extensions/compiler.ts` and `legacy/core/model.ts` are byte-identical between pins. Their provenance is audited, and each compiler/bundle pair is exercised independently. This does not certify every imported dependency or project conversion. |
| E04/E05/E16 and related effects members | Current-main adds version/default helpers, effect registration metadata and capability flags. Those E rows go to delta-fx, as directed. No unsupported flag was enabled here; native effects/recorder/resource gaps remain visible. Async render siblings remain the recorded native superset, not an invented current-main public member. |
| U02/U08 and other U rows | Axis increments/grouping, fitContent and menu presentation go to the later UI lane. No visual/interaction choices were made here. P-row data work remains with delta-data. |

`node tools/extcheck/source-audit.mjs` emits source-only hashes for the eight
named carry/compiler/factory files; its regression checks both immutable Git
objects. It explicitly reports `native_acceptance: false`. No shared delta
inventory was rewritten, and no byte-identical source finding closes a native
gate (**N3**, source evidence and authored behavior kept distinct).

Current reference-change checks: **40 Python pass**, **3 reference/source/pair
regressions pass**, clock preparation/startup **8 pass**, real V8 clock **3
pass**, and the bundle regression passes separately for both pins. Updated Mac
commands and remaining implementation blockers are in the area handoff.

One transport boundary changes at the new pin: `legacy/core/engine.ts` now
pauses on the last actually displayed input-preview time when its project and
render-content generation still match. The worker's existing basic seek/clock/
loop tests cover the non-preview path; they do **not** establish that presented
frame behavior. Real renderer presentation feedback is still unattached in
this runner. Keep this E18/transport integration boundary explicit; do not
substitute clock time for an acknowledged preview or advertise that it passes
(**N3**, authored displayed-frame intent). The presentation/engine lane needs
to supply that feedback before full-script acceptance.


Final refresh merges `78294c4c93cdb2a3a18aef8004fc42f9e5e420a9` without
conflicts. Post-refresh counts: locked native runner **24**, portable host
**20**, Store host **90**, Python **40**, preparation/startup/reference Node
**11**, real V8 clock **3**, all passing. Clippy/fmt remain clean. The prior
intermittent profile-reopen failure stays recorded, and P2 remains OPEN.

## 2026-11-05 — ext4 native resource and Store connections (P2 still open)

This lane adds production connections within the extension-owned crates. It does
not record a new accepted comparison: **0/58 complete comparisons accepted**.
Current-main remains pinned to `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`;
legacy inputs and the identical deterministic clock fixture in both hosts remain
required. No private source, corpus identifier, project, selector or screenshot
was added. No visual design decision or new DESIGN-TODO(opus) was introduced.

- **Native Canvas → DOM:** the actual V8 runner binds DOM canvas nodes to native
  recorder surfaces, retaining DOM node/context identity and same-size resize
  reset. Immutable PNG frames carry the exact source version and DOM sequence.
  `CompatRunner::canvas_frames()` flushes bindings and freezes pixels on the
  worker. The shell owner must enqueue DOM batches before these frames.
  `MacDomHost::present_canvas` queues resource IDs; bounded, generation-scoped
  native resources are fetched separately from the DOM mutation protocol. The
  bundled page decodes/replaces the bitmap before releasing its resource or
  polling further work. Refused admission does not evict an older resource.
  Disconnected canvases leave the binding set; their held native sources survive.
  The app must still wire these queues; this is not full-window presentation.
- **Recorder bindings:** quadratic/cubic paths, arcs/ellipse/arcTo, six-component
  transforms, dashes, stroke caps/joins/miter, point tests, smoothing state and
  native PNG encoding now use the existing recorder. PNG decode tests check the
  actual pixel readback, including translucent pixels; held frames survive
  repaint and reset. Other overloads, typed ImageData, paints/text/shadows,
  resources, JPEG and engine overrides remain explicit unsupported backends.
  This is a connected subset, not a full HTML Canvas implementation.
- **Actual page capture endpoints:** `inspect_capture` reads one rendered node
  through host-authored JS; `capture_png` uses WebKit's snapshot completion and
  native PNG encoding. Both return worker-polled tickets and reject stale host
  generations. Neither blocks AppKit waiting for JS. The remote-DOM smoke example
  now verifies the rendered output and real PNG completion, with an optional
  second argument to save the PNG. A rendering-document snapshot does not replace
  a full-app screenshot, scripted native input or a complete behavior trace.
- **Store:** `KernelStoreAdapter::install_archive` connects pinned signature/hash
  verification and immutable archive preservation to the real durable kernel CAS,
  returning exact verified virtual files to its compiler caller. It does not
  invent publisher pins, old installer provenance or grants. `KernelStoreDispatch`
  connects bounded command registration/disposal to the kernel's provider stacks,
  qualifies IDs under the owning extension, refuses foreign owners and validates
  command schemas. Callback continuations run back in the confined helper after
  returning from the kernel call. Other registration kinds/view/resource/network
  dispatch still fail explicitly. Public catalog hashes alone remain insufficient
  for admission; the four public releases have not run through this installer.

Decisions: **N3/N6** select actual native pixels and recorder semantics rather
than another page renderer; no tolerance, masking or Chromium quirk was added.
**N7/N8** select inspectable generation/node/resource IDs, nonblocking UI admission
and worker-polled capture tickets. **N4/N6** select publisher-pinned Store admission,
owner-scoped provider stacks and confined callbacks; unsupported methods remain
visible. No existing consumer was pruned.

### PID 0 profile-reopen regression: deterministic reproduction, not fixed

The locked runner sweep reproduced **26 pass / 1 FAIL** at clean profile reopen,
with `Library(AsCopy(Locked))` and the synthetic PID 0 record. Later ordinary
sweeps passed; that does not resolve the failure. The new owned diagnostic runs
before V8 initialization, acquires the production `pm_kernel::locks::FileLock`,
forks a child that only waits on a pipe, drops the parent lock (joining its
heartbeat), then attempts immediate reopen while the child still holds the
inherited descriptor. It deterministically returns **FAIL / WouldBlock / PID 0**.
It always releases and reaps its child before reporting the failure.

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner --example lock-inheritance
```

This establishes the retained open-file-description mechanism independently of
V8, Bun or a particular helper spawn implementation. Close-on-exec does not
release an inherited descriptor before exec. The kernel must explicitly unlock
the stable inode after the heartbeat's final write and before dropping its
locked descriptor, including safe error paths. An explicit unlock is distinct
from unlinking the lock file, which remains forbidden. `kernel/src/locks.rs` is
outside ext4 ownership; it was not edited. No retry, serialization, unlink or
assertion relaxation was introduced. The diagnostic intentionally exits 1 on
the current kernel and is not counted as a pass.

### Checks before the integration refresh

All Cargo test/build/check/Clippy commands use `--locked`. Tooling was provisioned
outside the repository after this container lacked the requested activation file.
The lock update adds only this runner's dependency edges; no dependency version
was upgraded and the engine lockfile is unchanged.

- `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner -p pm-compat-host`:
  **29 runner + 23 host pass** on the latest ordinary sweep; the earlier reopen
  failure and deterministic failing diagnostic remain visible above.
- `bun test apps/native/crates/compat-host/tests/canvas-page.test.ts`:
  **2 pass / 11 assertions**, using the real PageApplier and a browser contract
  double. Checks decode-before-release/poll and refusal of a future DOM sequence;
  this is not WebKit pixel evidence.
- Package all-target Clippy (`--no-deps -- -D warnings`) and fmt pass on Linux.
  The host also passes macOS ARM64 cross-target checking/Clippy, including probes;
  this proves Rust typing only, not macOS linking or runtime behavior.

Remaining before all-58 acceptance: app-shell full script/trace/window-capture
wiring (outside this lane), reviewed private scenarios/projects and input snapshot,
real native input, complete recorder/engine/Store dispatch, authentic release or
preserved installer provenance, and the kernel lock correction above. The existing
single-command extcheck driver still refuses its unattached native full capture;
these component endpoints must not change that status to `captured`.

### ext4 final integration refresh and exact validation

Merged/refreshed base: **`5bcd425ddc158cbd891ee4b515cf9993fad29a66`**.
The final explicit remote-tracking fetch and merge reported already up to date,
with no conflicts. No other branch was written. Final source verification follows
`f9ddff52d` (the Canvas direction/winding contract correction).

The current-main source audit also selects two command details: programmatic
`runKernelCommand` directly invokes `run`, independently of eligibility `when`;
the guarded command reports callback failures and returns undefined. The real
confined-helper regression covers both. Canvas direction conversion and invalid
winding rejection are separately tested against actual pixels (**N3/N6**).

| Exact command from checkout root | Final result |
| --- | --- |
| `cargo build --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner -p pm-runtime-v8 --bins` | Pass; includes the real confined helper |
| `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner -p pm-compat-host -p pm-store-host -p pm-recorder -p pm-recorder-raster -p pm-remote-dom` | **295 pass / 0 fail / 0 ignored**: runner 30, host 23, Store 90, recorder 94, raster 36, remote DOM 22 |
| `bun test apps/native/crates/compat-host/tests/canvas-page.test.ts` | **2 pass / 11 assertions** |
| `python3 -m unittest discover -s tools/extcheck -p 'test_*.py'` | **40 pass** |
| `node --test tools/extcheck/test_clock_preparation.mjs tools/extcheck/test_startup_clock.mjs tools/extcheck/test_references.mjs` | **11 pass / 0 skipped** |
| `PM_EXTCHECK_RUNNER="$CARGO_TARGET_DIR/debug/pm-compat-runner" node --test tools/extcheck/test_clock_v8.mjs` | **3 pass / 0 skipped**, real V8/DOM/transport recordings |
| `node tools/extcheck/test_clock_fixture.mjs` | Pass; shared time/entropy/scheduler fixture |
| `cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner -p pm-compat-host --all-targets --no-deps -- -D warnings` | Clean |
| `cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-compat-host --target aarch64-apple-darwin --all-targets --features macos-probes --no-deps -- -D warnings` | Clean cross-target typing, including actual capture endpoints and probes; no macOS runtime claim |
| `cargo fmt --manifest-path apps/native/Cargo.toml -p pm-compat-runner -p pm-compat-host -- --check` | Clean |
| `cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner --example lock-inheritance` | **FAIL / exit 1 / WouldBlock / PID 0** again; production kernel defect remains |

The ordinary suite's successful reopen is retained alongside the earlier failed
sweep and this deterministic failure. It is not evidence of a kernel fix.
Gitleaks staged scans (with the repository's `--ignore-gitleaks-allow` requirement)
pass before each commit. No failure, ignored case, acceptance denominator or
pixel tolerance was relaxed.

New Mac component smoke, after installing the remote-DOM package's frozen
build dependencies (no V8 artifact is needed for this particular host-only probe):

```sh
bun build apps/native/crates/compat-host/tests/page-entry.ts --target=browser --format=iife --outfile=/private/tmp/pm-ext4-page.js
cargo +1.96.1 run --locked --manifest-path apps/native/Cargo.toml -p pm-compat-host --features macos-probes --example remote-dom-smoke -- /private/tmp/pm-ext4-page.js /private/tmp/pm-ext4-smoke.png
```

A real Mac must run that smoke and verify native PNG resource decode, transparent
pixels, scaling/orientation, repaint/reset/disconnect, resource pressure and stale
generation rejection in the actual WebKit process; verify snapshot failure and
host-close/recovery completion; and wire/present real extension frames through
the app shell. The complete gate still needs full-window geometry/fonts/theme,
actual keyboard/IME/pointer/AX interactions, complete behavioral instrumentation,
reviewed per-extension scripts/projects and three full recordings per role for
all 58. Cross-target Clippy proves none of those runtime properties.

**Finish status: BLOCKED, P2 OPEN.** Required changes outside ownership are the
explicit kernel lock release and app-shell/full-driver integration (including
`tools/extcheck/native.mjs`). Within the extension area, non-command Store
registration/view/resource dispatch (including GLSL effects), missing recorder
capabilities and real archive provenance remain open. The existing driver must
continue reporting the unavailable full adapter; no component probe may be
substituted for an accepted comparison. No new DESIGN-TODO(opus) was required.
