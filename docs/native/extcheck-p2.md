# P2 extension checks — 2026-11-01

**OPEN / NOT PASSED.** No extension has accepted paired Electron/native traces,
visual oracles and complete scripted-interaction coverage. Successful activation
and component tests are not extension matches.

Reference: frozen `v1.2.0`, `07ed5fa17ee640659cb075f9cad1af2095011d60`.
The earlier check's executable and harness were brought forward from its branch;
they were not present in the fetched native-engine branch at continuation time.
The S1 sandbox archive and bindings passed all five checksum checks. Builds use
Rust 1.96.1, `engine/env.sh`, the shared target and the build lock. No release,
app update, tag or deployment was created.

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
