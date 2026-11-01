# S5 — compatibility page architecture spike

**OPEN / NOT PASSED. Failures and missing evidence must go to Jude before P1 sizing.**
This is a synthetic transport/host spike, not extraction of the compatibility
kernel and not proof that the v1–3 corpus works.

## Contract and build

Read execution baseline v6 in full, S2's composition report and the landed
C-MATRIX. S5's WKWebView is a future foreign-view slot under S2's composition
contract; this standalone AppKit host does not implement GPUI composition,
overlay holes, foreign accessibility children or reflow. Those S2 gates stay open.

Rules determining this implementation:

- N4/N6 and §3.6: no v1–3 exclusions; live objects/callbacks stay in their realm.
  Synthetic probes do not certify compatibility or frozen-reference parity.
- N8: synchronous callers block only WebContent. The host's scrub timer never
  enters or waits on JS. HTTP/WebSocket work lives on separate threads. Bootstrap
  diagnostics use one asynchronous evaluate call; it is not a hot-path transport.
- §3.5.2: try HTTP first, then scheme handler, then text-input delegate. Selection
  requires p95 ≤0.5 ms **and** IME, accessibility and blocked-page snapshot checks.
- §3.5.2: lifecycle selection requires ≥95% of foreground export throughput after
  long minimise, hidden and windowless, on every supported OS. Locked-session
  observations cannot satisfy that gate.
- §3.2.1: acknowledged operations survive cancel and owner failure; history.cancel
  keeps edits and drops the entry. C-MATRIX distinguishes these native guarantees
  from literal shipped rollback behavior.

`apps/native/spikes/s5` is package `pm-native-s5-spike`, automatically included by
`spikes/*`. No workspace-member edit is needed. An empty `crates/*`
workspace glob initially prevented Cargo loading. Initial channel/pixel runs
used the exact sources with a temporary isolated manifest/lock, leaving shared
files untouched. The core/migrate lanes have since populated crates/* and the
shared lock now includes S5. **Normal workspace metadata and build now PASS**
with Rust **1.96.1**. The temporary workaround and redundant S5 lock are removed;
check.sh uses `cargo --locked -p pm-native-s5-spike`. The script sources
engine/env.sh, runs from apps/native, uses only `~/Developer/pm-target`, and
checks ≥4 GiB free before/during builds. No cargo clean or other target directory.

```sh
apps/native/spikes/s5/check.sh build
python3 apps/native/spikes/s5/run.py http scheme prompt app
```

Logs are outside Git in `/tmp/s5-*.jsonl`. The server binds only 127.0.0.1 on an
ephemeral port, generates a random per-run URL capability, checks Host and HTTP
Origin, emits no capability URLs and sends no CORS grants. The fixture contains
no user/project data. It is a spike, not production server hardening.

## Measurement conditions and limits

macOS 27.2 (26B5086k), Apple Silicon, **screen locked**. A logged-in GUI session
exists; WebKit processes and a page can start. No unlock was requested. This is
not a no-GUI-session test, an unlocked display timing run, or a supported-OS
matrix. Other lanes share the machine/cache, so no quiet-window gate is claimed.

The AppKit NSTimer requests 120 Hz. Each callback calculates 5,000 synthetic clip
transforms repeatedly for ≥1 ms and changes native view geometry. Report actual
work and interval distributions, rather than claiming a real 120-Hz display.
The server and host use an unoptimized development build. Page latency uses
performance.now and nearest-rank quantiles after warmup. App→page results are
server-send→page callback→server-ack **round-trip upper bounds**, not invented
one-way measurements or clock-synchronized estimates. No evaluateJavaScript is
used to deliver these calls. Cross-origin isolation headers are **fixture-only**
to obtain finer clocks; they are not a production CSP/COEP decision and would
need compatibility review for trusted extensions loading arbitrary web assets.

## 1. Channels under scrub load

Each isolated candidate used 10 warmups and **2,000 measured calls** with
cross-origin isolation enabled (observed clock increment **0.020 ms**). Scrub
work p95 was 1.053/1.047/1.085 ms in HTTP/scheme/prompt runs. Actual timer interval
p95 was 9.652/10.251/9.823 ms: requested 120 Hz, with measurable scheduling jitter.

| Candidate | p50 / p95 / p99 (ms) | Result / decision |
|---|---|---|
| Sync loopback HTTP, same-origin XHR | 0.400 / **0.660** / 1.040 | **FAIL** observed p95 >0.5 |
| WKURLSchemeHandler on main | 0.080 / **0.100** / 1.060 | Latency PASS; backup after latest HTTP repeat below |
| Text-input prompt delegate on main | 0.060 / **0.060** / 0.080 | Latency PASS; later candidate, not preferred by rule |
| Page-held long-poll, 400 calls | 1.271 / **2.502** / 6.801 | Ack upper bound; slower observed path |
| Off-main loopback WebSocket, 400 calls | 0.271 / **0.575** / 0.717 | Provisional app→page choice; no hot-path eval |

**No production channel is approved.** Scheme/prompt measure immediate synthetic
native replies, not a completed kernel-actor integration. IME/real WebContent AX
and supported-OS evidence are missing. HTTP straddles the threshold across runs;
these shared-machine observations do not establish an intrinsic limit.
App→page numbers include JS and return-trip overhead. WebSocket is chosen
provisionally because its observed delivery+ack bound is lower; EXT-RASTER-1
still needs a real override workload.

### Final workspace repeat

After the normal workspace build became available, the final validation repeated
all six functional cases (HTTP, scheme, prompt, app→page, binary, continuation).
All completed with exit 0 and their integrity/proxy assertions passed. The final
driver also requires snapshot completion, checks all 400 WebSocket acknowledgments,
and parses the WebSocket upgrade through the authenticated HTTP path, including
partial reads. New log names include a run timestamp so earlier evidence survives.

| Sync candidate | Repeat p50 / p95 / p99 (ms), n=2,000 |
|---|---|
| HTTP | **0.360 / 0.400 / 0.420** |
| Scheme | **0.080 / 0.100 / 1.060** |
| Text-input prompt | **0.040 / 0.060 / 0.080** |

HTTP's p95 changed from **0.660 (FAIL) to 0.400 ms (PASS)**. Per §3.5.2's
first-passing rule, **HTTP is again the first provisional candidate**, with the
scheme handler as backup; do not freeze the earlier scheme preference. A stable
quiet-window run and real IME are still needed before choosing. Scheme's p99
around 1.06 ms is also recorded even though the stated gate uses p95.

App→page repeat (400 each): long-poll p95/p99 **2.571/3.223 ms**; WebSocket
**0.391/0.733 ms**, both acknowledgment upper bounds. **WebSocket remains the
provisional choice.** These are two diagnostic runs, not the ≥30 independent,
paired, quiet-window performance gate runs specified for full app budgets.

An initial timer-yielding fixture did not finish in 90.009 s; host scrub work p95
1.086 ms and interval p95 8.745 ms continued. A later 35-s external timeout also
occurred with timer yields. Removing page timer yields from the sample loop
allowed completion; suppression is suspected, not proven. These attempts are
**inconclusive harness/lifecycle observations**, not channel latency samples.
They are retained as a lifecycle risk, not hidden or called a deadlock proof.

The text-input candidate is JavaScript prompt → WKUIDelegate's text-input panel
completion. It answers immediately without presenting a modal dialog. Scheme
requests use a registered custom scheme from the HTTP page; protocol support/
origin failures count as failures, not as timing samples.

Deadlock fixture: focus a real input, block same-origin sync XHR for 300 ms on a
server thread, request takeSnapshot asynchronously from main, and enumerate the
native window's accessibility children. A DOM CompositionEvent sequence also
calls the sync endpoint. Native AX enumeration is only a container proxy; it
does not walk WebContent. Synthetic composition does not exercise an OS IME.
All three candidates returned from a 300-ms deliberate hold: HTTP **311.78 ms**,
scheme **300.90 ms**, prompt **301.12 ms**. Each had an async snapshot requested
while blocked, a non-null image/no error after release, a successful container AX
query (**0.294/0.461/0.332 ms**) and all three synthetic composition callbacks.
These limited deadlock proxies **PASS**; full OS IME and a real VoiceOver walk
remain **UNVERIFIED** while locked. Snapshot completion latency relative to its
request is not yet instrumented; no claim that snapshot completed during the hold.

## 2. Binary over sync

Payload: **8,294,400 bytes (1920×1080 RGBA)**. RGB exercises low and high byte values;
alpha is opaque to avoid premultiply/unpremultiply loss during canvas verification.
Each transfer checks every byte. One warmup, 30 measured transfers per direction;
2,000 ID operations. All numbers below use HTTP, the off-main reference path,
**not the provisional scheme handler**, and include synthetic Rust/JS validation.
These unoptimized results do not isolate transport copies from validation cost.

| Operation | p50 / p95 / p99 (ms) | Result |
|---|---|---|
| App→page x-user-defined response → decoded buffer + verification | 81.980 / **137.440** / 139.800 | Integrity PASS; too slow for 10-ms sync-pixel budget even before canvas fill |
| Page→app typed-array request body + native verification | 51.140 / **54.560** / 55.880 | Integrity PASS; synchronous upload does support binary request bodies |
| Page→app text string (UTF-8 on wire), native text decode + verification | 170.680 / **175.680** / 179.960 | Integrity PASS; much slower; text-string construction excluded |
| App→page engine-held reference | 0.720 / **0.920** / 1.340 | Reference correctness PASS; no pixel bytes crossed |
| Page→app consume engine-held reference | 0.820 / **1.140** / 1.660 | Reference correctness PASS; no pixel bytes crossed |

**Choice: keep pixels native and pass IDs whenever possible** (§3.5.2 preferred
option, N8). The synthetic frame really remains in Rust, and the ID resolves to
its buffer. IDs do not solve synchronous APIs that expose canvas pixels. That
route **FAILS the provisional 10-ms budget** in this fixture. Optimized builds,
scheme-handler binary transfer, real engine readback and frozen-reference paired
timings remain unverified; no exclusion from N4 follows from this result.

Final workspace repeat (30 each): download/decode/verify p95 **135.100 ms**,
typed-array upload/verify **51.240 ms**, text upload/decode/verify **171.280 ms**.
Reference-only download/upload p95 **1.160/1.440 ms** (2,000 each). Integrity
passed again; the architectural conclusion is unchanged.

## 3. Render continuations

**PASS synthetic no-deadlock/correctness; FAIL provisional latency budget.**
Inspected the pattern at
`apps/desktop/src/renderer/src/engine-connection/render-transport.ts:1215`.
The actual WebKit stack executes:

1. Synchronous outer render returns `needs_dependency`, render ID, revision 7.
2. JS runs a real Canvas2D raster override inline and makes a nested sync capture.
3. Nested capture returns a real 1080p canvas; its pixels are read and uploaded
   back to the native stub, which verifies every byte.
4. The 16×16 override pixels are uploaded under the pinned render/revision. Rust
   validates and copies them into its output buffer, then resumes the outer job.
5. Outer result is fetched through x-user-defined and filled with putImageData.
   Checks confirm the override appears in the returned real canvas. A real JPEG
   data URL is also produced in the correctness probe.

Observed trace: `outer needs_dependency → nested rendered → outer rendered`.
Unknown/stale/replayed resumes are rejected. The synthetic stub has fixed
revision 7 and one retained output; it does not implement production frame-ID
lifetimes, a real dependency graph or cancellation. No event-loop callback into
the blocked page is needed and the native scrub continues.

**30 measured nested captures:** p50/p95/p99 **260.120/357.240/371.740 ms**,
including two 1080p canvas fills, nested readback/upload, native pattern generation
and byte checks. This is a heavier synthetic operation than the plan's ordinary
paused renderFrameTo row, but plainly cannot certify ≤10 ms. The JPEG correctness
probe is outside this timed loop. Two Rust tests pass for byte corruption and
pinned/single-use/stale resume behavior.

Final workspace repeat (30): p50/p95/p99 **239.080/383.700/384.200 ms**;
canvas/JPEG/override integrity and stale-resume checks passed again. Pixel budget
remains failed.

**Decision:** use bounded same-stack continuations (N8, §3.2.1/§3.5.2), with native
IDs until pixels are demanded. Do not approve this pixel transport for production.
Real engine integration, cycle graph, override exceptions/cancellation,
frozen-Electron renderFrameTo+canvas timing and its stricter 70% margin remain
**UNVERIFIED**. C-MATRIX's reference-version mismatch also remains open.

## 4. Lifecycle, hang and restart

Synthetic page job: 1080p Canvas2D, 100 changing rectangles, a small readback
and one sync native request per frame, driven by the page for 5 seconds per
phase. This is not the production engine export workload. The host waits **60 s**
after requesting minimise and hiding the app, runs the hidden phase, then closes
the editor window and transfers the page into an offscreen host for the
windowless phase. No app→page eval wakes the job: a page-held request gets the
next phase from the off-main server.

| Activity option | Initial / after long hidden dwell / windowless (jobs/s) | Diagnostic result |
|---|---|---|
| 1. Disable page-visibility process suppression and hidden-page timer throttling (SPI) for the job | **883.087 / 653.375 / 610.946** | **73.988% / 69.183%** of initial, below 95%; selectors available and original preferences restored at end |
| 2. Always-visible offscreen host, canHide=false | Initial **786.858**; **no hidden completion before the 160.055-s host deadline** | FAIL to demonstrate progress after hide; no windowless sample. Host visible to AppKit, editor confirmed minimised after 60.001 s; page still reported hidden |
| 3. V8 trusted realm + remote DOM | **Not implemented / no numbers** | Fallback unvalidated; no proxy throughput invented |

The initial page already reported document.hidden=true because the screen was
locked. Therefore there is **no true foreground comparator**; the percentages
are diagnostics, not an export gate. SPI did complete hidden and closed-editor
jobs, but its observed relative rates also fail the provisional 95% threshold.
This is one job/OS, not ≥30 paired export runs across supported macOS releases.
No lifecycle option is approved. Initial isMiniaturized was false immediately
after the asynchronous minimise request; option 2's later check confirmed true.
The option 1 run preceded that later check and cannot certify its minimise state.

SPI names were checked against WebKit's primary source, and runtime selectors
were tested before calling them:
[WKPreferences private API](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKPreferencesPrivate.h),
[WKWebView private API](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKWebViewPrivate.h).

Infinite-loop fixture uses a real `while(true){}` after writing synthetic
localStorage and notifying native code. Only this explicitly requested test
automatically invokes Stop; product D11's user Stop/Wait policy is not implemented.
**Named SPI failure:** `_killWebContentProcess` returned but did **not** terminate
the owned busy content PID or send a termination callback within **20 s**, on the
corrected PID-observing run. No restart is demonstrated for that exact API. The
earlier disappearance after the host exited was cleanup, not successful Stop.
This is a failure for Jude; the cause (OS/API behavior versus host interaction)
is not established from this single OS/session.

**Additional diagnostic:** `_killWebContentProcessAndResetState` **did** stop the
real loop and restart successfully. One final probe: termination callback at
**0.824 ms** after the SPI call (old PID still present at that instant), new page
ready at **106.479 ms**, new positive/different PID, and **old PID confirmed gone**
before success. Real localStorage survived and the page fetched revision 7 from
the synthetic native snapshot endpoint. This is **not** production kernel/journal
recovery or reactivation of real extensions. It is not the plan's automatic
fallback: it is a separate SPI diagnostic. **Recommendation:** use the reset
variant as the provisional Stop implementation under N4/N8/D11, subject to Jude's
review of the named-API failure and supported-OS validation. No plan gate changes
or production approval are implied.

**Fallback:** with explicit SPI disabled, WebKit did **not** terminate the real
loop within the **20-second observation window**. Native scrub continued
(work p95 **1.057 ms**, interval p95 **8.956 ms**). This bounded fallback test
**FAILS to demonstrate termination/restart**; eventual WebKit intervention and
unlocked user prompts remain unverified. Exiting the fixture afterwards is
cleanup, not evidence that the fallback worked. V8 + remote DOM remains the
specified architectural fallback, **not an implemented or measured solution**.

## 5. Transactions / remaining work

**PASS: eight executable synthetic scenarios** in a minimal single-writer actor,
run with `pm-native-s5-spike --kernel` (no WebKit/window needed). Three Rust tests
pass overall, including this scenario suite and the two pixel/continuation tests.
The stub is deliberately separate from the production core and from the native
UI; its blocking CLI request shim is never called by AppKit.

| Scenario | Observed invariant |
|---|---|
| Lost ack, retry and txid query | Same stored answer; revision increments once; key includes realm/generation/txid |
| Rejection after optimistic page mutation / target deleted natively | Authority unchanged on invalid value; deleted-path flush returns failure + null resync target |
| Native drag + admitted page write + edit.cancel | Immediate folded acknowledgment; owner x reverts, foreign y remains in its own undoable entry |
| Overlapping property writes | Conflicting owner inverse → commit with notice; acknowledged foreign value kept |
| Owner create + foreign edit dependency | Cancel cannot delete the foreign writer's layer; commit with notice |
| Owner delete + foreign recreate/edit dependency | Cancel cannot overwrite acknowledged recreated data; commit with notice |
| history.cancel | Edits remain, pending history entry is dropped |
| Begin left open + UI/MCP/agent writes + owner failure | Admitted writes keep folding without waiting; edit.mutate refuses live source edit with ok:false; failure seals acknowledged owner/foreign operations into undoable history |

**Choice:** stage and validate owner inverses against current authority; if any
conflict, keep the whole entry with notice. Otherwise split foreign operations
into their own entry. This is the simple conservative option satisfying
§3.2.1's acknowledged-write invariant; it intentionally favors keeping work.
It is not a port of every write API's return/admission semantics.

The lost-ack scenario simulates a lost reply with retry/query, not a faulted real
socket. Owner failure is an actor command, not a real realm crash or journal
recovery. Retained JS references, observer/event ordering, actual write-barrier
flush timing, external-undo continuation versus concurrent page commands,
durability/recovery and frozen-reference traces remain **UNVERIFIED**. No model
pass is presented as a native race or compatibility gate.

This pass also does not close extraction, inspector/cluster switch, recorder
promotion, Store-process isolation, virtual time, presenter or 39-extension memory
gates; these are outside the architecture-first implementation in this report.

## Current failures for Jude before P1 sizing

- HTTP channel p95 straddles the 0.5-ms limit: 0.660 initially, 0.400 ms on repeat.
  No stable channel selection is approved from these shared-machine runs.
- Sync HTTP pixels and nested-capture canvas path greatly exceed 10 ms; IDs
  preserve native buffers but cannot replace observable canvas/pixel APIs.
- SPI activity diagnostic ratios are 74.0%/69.2%; offscreen-only hidden job does
  not complete before 160.055 s. There is no true foreground baseline while locked.
- Exact `_killWebContentProcess` and automatic WebKit termination did not stop
  the real loop in 20 s. Reset-state SPI works in one probe; requires a decision.
- Full IME/VoiceOver, kernel-actor scheme round trip, quiet-window repeats,
  frozen-reference paired traces/timings, supported OS runs and the V8+remote-DOM
  fallback remain unverified. No compatibility exception or P1 approval follows.

## Reproduction / validation handoff

```sh
apps/native/spikes/s5/check.sh build
apps/native/spikes/s5/check.sh test
~/Developer/pm-target/debug/pm-native-s5-spike --kernel
python3 apps/native/spikes/s5/run.py http scheme prompt app binary continuation
python3 apps/native/spikes/s5/run.py kill-spi kill-fallback kill-reset
python3 apps/native/spikes/s5/run.py lifecycle-spi lifecycle-offscreen
```

The driver runs one instance at a time, returns nonzero for fixture errors or
watchdog expiry and keeps all raw logs outside Git. Expected failing probes are
not converted into successful exits. Screenshots, corpus data, private baseline,
capability URLs and local operational logs are not committed. The trusted-realm
V8 + remote-DOM fallback is still a required unvalidated option, not dropped.
No release/tag/push or P1 approval was performed.

## Unlocked rerun (2026-11-01)

**Verdict: OPEN / NOT PASSED.** The unlocked run confirms substantial pixel,
continuation and lifecycle failures. No production channel/activity choice or P1
approval follows. Earlier locked-session evidence remains above.

### Foreground conditions and harness correction

macOS 27.2, Apple M5, unlocked built-in display; one spike instance at a time,
unoptimized Rust **1.96.1** build in `~/Developer/pm-target`, shared machine.
The normal workspace build initially passed. Rebuilds used a temporary copy of
the manifests/lock pointing at these exact sources while another workspace member
was incomplete; the shared manifests/lock were unchanged. The final build passed.

The old host only ordered its accessory window front. The first unlocked attempt
called activation but logged AppKit state before its asynchronous completion.
The harness now **waits for app-active AND key-window before navigation** (up to
2 s, otherwise it fails explicitly). Every accepted probe logged app_active=true,
window_key=true, visible=true and minimized=false before starting. Page startup
also recorded document.hidden=false and document.hasFocus()=true. The confirmed
runs below are the decision evidence; earlier attempts are retained in `/tmp`
as diagnostics. Every candidate, pixel test, continuation, lifecycle option and
hang mode was repeated after the correction.

The screen automatically locked at **00:42:38** during an offscreen-host repeat.
That repeat's hidden/windowless samples and subsequent rejected starts are
excluded from unlocked evidence. The foreground guard correctly refused to run
the latter. After the owner unlocked the Mac, the offscreen-host and all hang
modes were repeated. Temporary activation experiments made while diagnosing the
lock were removed; no parent app identity/activation workaround is retained.

Main requests 120-Hz synthetic scrub work; this is still a timer workload, not
actual engine scrubbing or a 120-Hz WebKit presentation proof. Sync candidates
used 10 warmups and 2,000 measured calls, with 0.020-ms observed clock steps.
One run per case is diagnostic, not a quiet-window or supported-OS performance gate.

### Channels and blocked-page probes

| Channel | p50 / p95 / p99 ms | Updated result |
|---|---|---|
| Same-origin sync HTTP | **0.440 / 0.780 / 1.240** | **FAIL** p95 ≤0.5 ms |
| Main-thread scheme handler | **0.080 / 0.100 / 1.060** | Latency **PASS**, first provisional candidate in this run |
| Text-input prompt delegate | **0.060 / 0.080 / 0.160** | Latency **PASS** |
| Long-poll app→page ack, n=400 | **1.019 / 1.881 / 10.891** | Correctness **PASS**; round-trip upper bound |
| WebSocket app→page ack, n=400 | **0.308 / 1.057 / 2.895** | Correctness **PASS**; lower p95 upper bound |

HTTP/scheme/prompt scrub CPU work p95 was **1.069/1.086/1.062 ms**; actual timer
interval p95 **9.763/9.770/9.581 ms**. App→page scrub work/interval p95 was
**1.060/8.357 ms**. WebSocket max ack was **30.829 ms** (long-poll **14.063 ms**),
so its lower p95 must not conceal its tail. All 400 messages were acknowledged
on each path. No hot-path evaluateJavaScript delivery was added.

Deliberate blocked calls returned at **307.400/300.600/301.400 ms** for
HTTP/scheme/prompt. Each requested a snapshot while blocked and completed with
non-null image/no error. Container AX proxy queries took **0.274/0.393/0.297 ms**;
all synthetic composition events passed. These remain **limited proxy PASS**,
not OS IME or VoiceOver deadlock certification. An external trusted AX walk of
the visible host exposed native window/container controls but no WebContent tree.
Snapshot completion during the hold (rather than after release), real IME and a
full foreign-child AX walk remain **UNVERIFIED**. No production sync channel is
approved; HTTP still straddles the threshold across reports.

### 1080p pixels and render continuations

Payload remains **8,294,400 bytes**, one warmup plus 30 measured transfers per
direction; each verifies every byte. IDs use 2,000 measured operations.

| Operation | p50 / p95 / p99 ms | Updated result |
|---|---|---|
| App→page text response/decode/verify | **82.380 / 91.920 / 92.140** | Integrity PASS; **FAIL** 10-ms pixel budget |
| Page→app typed-array body/verify | **56.020 / 72.060 / 85.940** | Integrity PASS; **FAIL** provisional pixel budget |
| Page→app UTF-8 text/decode/verify | **161.600 / 230.120 / 258.560** | Integrity PASS; much slower |
| Engine-held ID download | **0.380 / 0.460 / 0.580** | Correctness PASS; no pixel bytes crossed |
| Engine-held ID consume/upload | **0.440 / 0.500 / 0.580** | Correctness PASS; no pixel bytes crossed |
| Nested capture + outer canvas fill, n=30 | **259.100 / 286.680 / 348.560** | Continuation correctness PASS; **FAIL** 10-ms budget |

The real canvas/JPEG, override pixels, pinned/stale/single-use checks passed again.
The trace was `outer needs_dependency → nested rendered → outer rendered`.
Binary scrub CPU/interval p95 **1.067/9.070 ms**; continuation **1.056/9.648 ms**.
Unlocked execution improves some rows but does not resolve the architectural
pixel cost. Keep pixels native and pass IDs when semantics permit. Real engine
integration and frozen-Electron paired canvas timing remain unverified.


### Lifecycle: true foreground versus minimize/hide and closed editor

The accepted SPI run completed before the screen lock. The final offscreen run
completed after the owner unlocked the session; session state was checked unlocked
again afterwards. Each phase performed the same five-second synthetic Canvas2D +
sync-request job. Both waited **60.00 s** after minimize/hide. The delayed checks
confirmed editor_minimised=true and app_hidden=true. Closed-editor checks confirmed
editor_visible=false. Pages reported hidden=true in both background phases.

| Activity option | Foreground / hidden / windowless jobs/s | Hidden / windowless share | Result |
|---|---|---|---|
| Job-scoped WebKit suppression/timer SPI | **1172.726 / 333.177 / 326.656** | **28.411% / 27.854%** | **FAIL ≥95%** |
| Always-visible offscreen host, canHide=false | **949.346 / 363.098 / 386.943** | **38.247% / 40.759%** | **FAIL ≥95%** |

Both options completed all phases unlocked. This resolves the earlier offscreen
non-completion observation on this OS, but throughput remains far below the rule.
The SPI run logged activity release and restored its original WKPreferences.
The offscreen host stayed AppKit-visible while the page still reported hidden.
Work/interval p95 was **1.108/8.940 ms** for SPI and **1.127/9.144 ms** for the
final offscreen run. This is not production export, a supported-OS matrix or a
validated V8 + remote-DOM fallback. **No activity option is approved.**

### Real infinite loop and restart, final unlocked batch

Every final hang mode began with app-active/key/visible state confirmed and a
visible, focused page. A real `while(true){}` ran in WebContent.

| Stop path | Updated result |
|---|---|
| Exact `_killWebContentProcess` | **FAIL**: no termination/restart within the **20-s** loop observation window; host exit 2 at **20.509 s**. Selector exists and call returned. |
| WebKit automatic termination fallback | **FAIL to demonstrate termination** within **20 s**; host exit 2 at **20.203 s**. No eventual-termination claim. |
| `_killWebContentProcessAndResetState` diagnostic | **PASS synthetic recovery**: callback/observation **0.975 ms** after Stop; replacement ready **98.648 ms** after Stop; different positive PID and old PID confirmed gone. localStorage survived and native snapshot revision **7** was fetched. |

The first observation in the reset path preceded old-PID exit; success was only
reported after confirming its exit. Failed-host cleanup is not counted as Stop.
Scrub work/interval p95: exact SPI **1.050/9.076 ms**, automatic fallback
**1.059/9.131 ms**, reset variant **1.058/9.112 ms**. The host remained responsive.
These explicitly requested test kills do not implement product Stop/Wait policy,
production gesture sealing, kernel/journal recovery or extension reactivation.
The reset variant remains a separate provisional diagnostic requiring a decision;
the exact named API and automatic fallback remain failed.

### Artifacts, checks and final gate

Screenshots outside Git:

- `/tmp/pm-shots/s5v-confirmed-foreground.png`: visible foreground probe after activation confirmation.
- `/tmp/pm-shots/s5v-foreground-lifecycle.png`: earlier unlocked page baseline.
- `/tmp/pm-shots/s5v-real-loop-confirmed.png`: final unlocked real-loop batch.

Accepted raw logs are listed by `/tmp/s5v-confirmed-driver.jsonl` (six foreground
cases and SPI lifecycle) and `/tmp/s5v-unlocked-final-driver.jsonl` (final offscreen
and three hang modes). The lock-interrupted offscreen repeat and all rejected
foreground starts remain separate diagnostics, not accepted timing samples.
All six functional cases and both accepted lifecycle probes exited 0. Exact SPI
and automatic fallback correctly exited 2; reset-state exited 0. The final pinned
build and **all three S5 Rust tests PASS**. The shared workspace's later locked
build was refused because its lockfile was changing in another lane; no lockfile
change was made here. Temporary manifest builds/tests used the exact sources.

**S5 remains OPEN / NOT PASSED:** HTTP latency, pixel/canvas budgets, ≥95% lifecycle
throughput and the exact hang path fail. Scheme/prompt latency, transport integrity
and reset-state diagnostics passing do not close real IME/VoiceOver, production
kernel integration, compat extraction/corpus, frozen-reference, supported-OS or
V8-fallback gaps. These failures remain required input before P1 sizing.

All spike processes were stopped. The GUI lock was released after every batch.
WKPreferences activity overrides were restored. System-level accessibility/input
restoration details, including the explicit-false preference-key limitation, are
recorded in the S2 rerun section. The owner's caffeinate assertion was left intact.
No screenshot, private plan, capability URL or raw log was committed; no push,
release or tag was performed.

## S5b (2026-11-01)

**Verdict: S5 remains OPEN. Evaluate the V8 trusted realm + remote-DOM fallback
when S1 is available.** Engine-held IDs remove the bulk control-channel traffic,
but they do not make newly rendered pixels synchronously available to a real
WebKit canvas. This is a measured synthetic architecture spike, not a production
renderer, compat-kernel extraction, corpus certification or permission to drop a
v1–3 API.

### Conditions and decision rules

The screen remained locked. `S5_HEADLESS=1` bypasses activation entirely: a
windowless `NSApplication` hosts the WebView in an offscreen, initially unordered
window. AppKit reported inactive/not key and the page reported hidden/unfocused.
No unlock, screen capture, input automation or foreground-session claim. This
still requires a logged-in WindowServer session; it is not proof of no-GUI CI.
macOS 27.2 (26B5086k), Apple M5; pinned Rust 1.96.1, unoptimized development
build, one S5 host at a time, shared machine. The native 120-Hz requested timer
continues the same ≥1-ms synthetic scrub work. Other lanes compile concurrently;
these are diagnostic runs, not quiet-window, paired performance gates.

- **N4/N6, §3.6:** preserve real canvas, synchronous returns, versioned sources,
  pixel observations and JPEG data URLs. A warmed-up canvas or an async sibling
  cannot substitute for an arbitrary synchronous capture of a new frame.
- **N8, §3.5.2 and orchestrator decision 3:** scheme-handler control requests;
  immutable native surfaces referenced by id/revision; same-stack continuations;
  no hot-path evaluateJavaScript and no raw pixels, JSON pixel arrays or base64
  images in continuation messages. The native UI never waits for page JS.
- **Selection:** channel p95 ≤0.5 ms under scrub; paused 1080p pixels including
  canvas fill ≤10 ms (or 70% of the frozen reference, if lower); EXT-RASTER-1
  JS ≤1 ms/layer and complete compat path ≤4 ms/frame p95; lifecycle ≥95% of
  actual foreground throughput after long minimise, hide and closed editors on
  every supported OS. A locked initial phase cannot certify that last rule.
- **D11/N4:** explicit Stop uses checked reset-state termination, then restores
  authoritative state. Never silently kill a trusted extension. Synthetic test
  Stop is explicitly requested here; product Stop/Wait and gesture sealing still
  belong in production integration.

### Pixel ownership, promotion and the synchronous boundary

`src/s5b.rs` owns immutable synthetic 1920×1080 RGBA surfaces. Control replies
contain only ids/revisions or dependency descriptors. Four recorder callbacks
return commands and a versioned nested source id in one batch. The new path has
no page→engine pixel upload; the old byte-transfer fixture remains solely for
comparison. IDs are revision-pinned and resumes are single-use; stale, unknown
and replayed requests are rejected by an executable Rust test. This bounded
fixture has fixed known commands and pre-rendered outputs; it does not measure
pm-render, general Canvas2D recording, memory reclamation or a full dependency
graph.

| Member | Chosen internal design | Observable boundary that still has to work |
|---|---|---|
| renderFrameTo / raster | Native surface id + content revision; batch recorder commands and nested source ids | Return the same real canvas/context objects. On pixel demand, materialize the correct version into its backing store. A new cold surface must work synchronously. |
| snapshot | Keep the source/render native; encode from that surface where possible | Return the required JPEG data URL synchronously, including page canvas edits. Native JPEG serialization is future work; returning an id is not API parity. |
| raster overrides | Inline page callback records commands referencing immutable source versions; engine consumes them directly | A callback using getImageData, toDataURL or real WebGL pixels needs promotion. Such calls cannot be silently deferred. |

Promotion candidates use native ImageIO PNG/TIFF or uncompressed BMP resources,
served by HTTP or `WKURLSchemeHandler`, then real `<img>.decode()` + drawImage
or fetch→Blob→createImageBitmap→drawImage. The final scheme implementation wraps
an immutable `Arc<Vec<u8>>` in NSData with a retaining deallocator: **no extra
NSData copy in the UI process**, not a claim of zero-copy WebKit IPC or decode.
A 64-KiB didReceiveData streaming variant is also measured. Custom-scheme image
responses require CORS/CORP headers plus `crossOrigin=anonymous`; without them
load succeeds but canvas readback throws SecurityError. Fixture-only grants do
not decide production asset permissions.

Both a compressible pattern and deterministic noisy opaque RGB are tested;
full 8,294,400-byte `<img>` integrity checks run outside timing, with a 1-pixel readback
inside each sample to force observable canvas work. ImageBitmap/WebGL use pixel
smoke checks; no whole-frame WebGL oracle is claimed. Native encoding/preparation
is reported separately and excluded from decode/fill timings: these are **lower
bounds** on a new engine frame, not end-to-end budget passes. Each candidate has
one warmup and 30 timed samples. Warm-fill rows reuse an already decoded image.

Final isolated image batch: **all 14 cases completed**, all 14 full `<img>`
byte checks passed. p95 milliseconds below (30 samples per cell):

| Transport / resource | Pattern img decode+fill | Noise img decode+fill | Pattern ImageBitmap+fill | Noise ImageBitmap+fill | Pattern / noise warm fill |
|---|---:|---:|---:|---:|---:|
| scheme / BMP | 31.96 | 19.26 | 139.62 | 35.16 | 2.72 / 2.42 |
| http / BMP | 30.44 | 24.30 | 132.96 | 174.90 | 2.46 / 1.86 |
| stream / BMP | 40.40 | 23.16 | 142.80 | 41.50 | 3.58 / 2.14 |
| scheme / PNG | 19.28 | 93.46 | 13.74 | 159.66 | 2.70 / 2.58 |
| http / PNG | 39.44 | 70.24 | 43.42 | 212.84 | 3.34 / 3.74 |
| scheme / TIFF | 21.30 | 21.08 | 112.34 | 137.78 | 3.20 / 1.78 |
| http / TIFF | 151.96 | 21.22 | 180.08 | 132.32 | 4.08 / 2.72 |

**No cold route meets ≤10 ms**, even with native preparation excluded. The
lowest observed p95 is 13.74 ms for the compressible PNG through scheme→Blob→
ImageBitmap; the lowest noisy-frame p95 is 19.26 ms for scheme BMP→img. These
are different candidates, not one robust passing design. Streaming BMP is
40.40/23.16 ms versus whole-resource 31.96/19.26 ms in this batch; it does not
rescue the budget. Warm pixels can be filled cheaply, but are already decoded.
A warm WebGL texture upload + fullscreen draw + gl.finish + drawImage into a
real 2D canvas (with pixel verification) has p95 **16.00–195.32 ms** across the
14 hosts, excluding image load/decode. It is not a cheaper promotion route here.
This does not certify EXT-WGL-1's different production workload.

Across the 16 final-batch host initializations (14 image cases plus continuation
and profile), native preparation p50/p95 was: pattern BMP **45.28/95.89 ms**,
PNG **24.77/41.40 ms**, TIFF **4.03/7.60 ms**; noise BMP **45.05/290.20 ms**,
PNG **265.09/695.69 ms**, TIFF **3.43/5.63 ms**. These unoptimized, contended
encoding/preparation numbers are separate from resource decode/fill and cannot
be added as paired quantiles. TIFF avoids the PNG compression cost but still
does not meet the decode/fill budget. Real engine render/readback is additional.

**Choice by the ≤10-ms rule: no eligible WebKit cold-pixel route.** Keep IDs for
engine consumers and the recorder path; async promotion may use image resources,
but it cannot be declared a replacement for synchronous v1–3 pixel APIs.

A fresh image tested on the same JS stack has `complete=false`, `naturalWidth=0`
and draws no pixels (transparent alpha 0). `img.decode()` and createImageBitmap
are Promise APIs: their successful asynchronous timings cannot prove synchronous
renderFrameTo semantics. This agrees with the [HTML canvas image-source
rules](https://html.spec.whatwg.org/dev/canvas.html). Preloading every possible
future render/revision is not a solution for nested capture or a canvas changed
by an arbitrary extension callback.

**IOSurface/shared-memory bound:** the supported [scheme-task data
API](https://developer.apple.com/documentation/webkit/wkurlschemetask/didreceive%28_%3A%29-8t5f8?language=objc)
accepts NSData byte resources, not a CGImage/IOSurface handle. No supported API
was found that imports an engine IOSurface or native mapped pointer into a page
CanvasImageSource/ArrayBuffer. Sending an IOSurface id in a URL or labelling raw
RGBA `image/x-…` does not add a WebKit decoder/importer. SharedArrayBuffer, even
where cross-origin isolation permits it, does not expose a host mapping API.
This is a bound on the interfaces investigated, not a proof about all possible
future/private WebKit APIs. No fake zero-copy path or browser-engine patch is
claimed. The initial custom BMP V4 resource was not decoded by WebKit; the final
BMP candidate uses conventional opaque BGRA32. Alpha/HDR promotion remains open.

### Continuation profile and redesign

The first locked reproduction split the old nested-capture fixture into these
stages (30 samples, ms p50 / p95). The individual p95s are not additive quantiles.
Native resume includes unoptimized synthetic pattern generation and is not a
pure channel hop.

| Old stage | p50 / p95 ms |
|---|---|
| HTTP begin | 1.84 / 4.08 |
| HTTP nested descriptor | 0.74 / 2.38 |
| Nested download + JS text decode | 76.40 / 108.40 |
| Nested putImageData | 0.86 / 3.92 |
| Nested full getImageData | 3.48 / 6.54 |
| Nested upload + native byte validation | 53.76 / 115.36 |
| Small override upload | 1.64 / 3.20 |
| Resume + native synthetic render | 50.38 / 76.62 |
| Outer download + JS text decode | 76.62 / 103.12 |
| Outer putImageData | 0.76 / 3.12 |

The 287-ms unlocked result was not mostly event-loop continuation delay. This
profile exposes two 8-MB text decodes, a redundant nested readback/upload and a
Rust debug-build pattern generator. The redesigned control flow uses three
scheme calls: begin returns a batch of four dependency descriptors; callbacks
resolve the nested version by id and record commands inline; one resume submits
the whole command batch and returns the outer id. No JSON/base64 serialization
or asynchronous callback into the blocked page is needed. No claim that JSON
alone accounted for hundreds of milliseconds.

Final control/continuation batch under the combined job activity settings:

| New path | n | p50 / p95 / p99 ms | Budget interpretation |
|---|---:|---|---|
| Scheme ID request | 2000 | 0.08 / **0.12** / 1.08 | PASS synthetic ≤0.5-ms channel latency |
| Four recorder callbacks + nested ID + batch resume | 2000 | 0.24 / **0.48** / 1.32 | Control-only lower bound; not full EXT-RASTER-1 |
| Same continuation + predecoded real canvas fill | 100 | 0.72 / **1.90** / 2.24 | Warm-case diagnostic ≤4 ms; no production raster gate claim |
| Same continuation + new image decode + real canvas fill | 30 | 19.36 / **117.26** / 117.58 | FAIL ≤10 ms; asynchronous lower bound on the cold operation |
| Full 1080p getImageData from materialized canvas | 30 | 2.36 / **5.00** / 5.06 | 5-ms readback; image load/render excluded |
| Materialized canvas to JPEG data URL | 30 | 13.50 / **14.84** / 14.94 | FAIL provisional ≤10 ms before render/decode |

Earlier diagnostic without the combined settings was 0.16-ms channel, 0.46-ms
ID continuation, 3.20-ms warm fill and 32.66-ms cold fill at p95. The final cold
117.26-ms tail must not be hidden behind that earlier 32.66-ms figure. Every
new canvas result checks the override pixel and produces real canvas backing;
JPEG output is a real JPEG data URL, not an id masquerading as API parity.
The native-only pattern-generator instrumentation on the final reproduction was
p50/p95 **48.89/66.33 ms** (30 samples after warmup), confirming that old resume
work was not just a channel hop.

**Decision:** retain same-stack, revision-pinned batched continuations with
engine-held ids (N8, §3.5.2). The control path is viable; **full EXT-RASTER-1 is
UNVERIFIED**, because the native result and command set are synthetic and
pre-rendered. Any override demanding new real pixels reaches the failed cold
boundary. No real-project JS≤1-ms/layer or renderer≤4-ms/frame pass is claimed.

### Background activity

Each candidate uses the same page-driven job: 1080p Canvas2D fill, 100 rectangles,
one small readback and one scheme ID call per iteration, five seconds per phase.
Additional one-second subtests separate JS compute, scheme-only, canvas-only and
batched-ID continuations. There is a 60-second dwell after minimise/hide and a
closed-editor phase hosted offscreen. Delayed checks confirmed minimised=true,
app_hidden=true, then editor_visible=false. **All phases report hidden=true;
“initial” below is locked/offscreen, never foreground.**

| Mechanism | Initial / hidden / closed-editor jobs/s | Hidden / closed share of locked initial | Diagnostic |
|---|---|---|---|
| Default | 869.79 / no completion / — | — | Host deadline 160 s; no hidden-job completion |
| Public inactiveSchedulingPolicy=None | 698.08 / 840.15 / 931.87 | 120.35% / 133.49% | Makes progress; candidate for a real foreground comparison |
| UI-process beginActivity(UserInitiated \| LatencyCritical) alone | 760.68 / no completion / — | — | Host deadline 160 s; does not protect WebContent by itself |
| Public policy + page suppression/timer SPI + UI activity | 795.62 / 545.72 / 772.50 | 68.59% / 97.09% | Hidden diagnostic below 95% |
| Same combined settings + AppKit-visible 1×1 offscreen host, canHide=false | 676.71 / 608.72 / 720.44 | 89.95% / 106.46% | Hidden diagnostic below 95%; page still hidden |

The public policy affects the actual WebContent scheduling preferences, whereas
NSProcessInfo's assertion here belongs to the UI process. WebKit's
[implementation](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKPreferences.mm)
shows that None disables background RunningBoard throttling and retains a
near-suspended assertion. The [public header](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKPreferences.h)
marks it macOS 14+, matching the deployment floor. This is a better diagnostic
candidate than pretending an app activity token applies to another process.
Both original policy and SPI preferences are restored and the activity token is
ended when the job finishes. Deadline exits also release process-owned resources.

Ratios alone hide substantial changes. In the combined run, JS subtest throughput
was 38,052→18,890→21,976 iterations/s, scheme-only 7,211→3,974→4,957,
canvas-only 2,306→1,475→1,743 and batched IDs 2,258→1,509→1,411.
Policy-only mixed throughput increased while its JS subtest fell from 37,130 to
23,542/22,914 iterations/s. Machine contention, workload mix and lifecycle all
matter; this sample does not isolate an intrinsic WebKit suppression ratio.
The 1×1 host does not supply a real visible page and cannot stand in for a
foreground baseline.

**Decision:** use public inactiveSchedulingPolicy=None as the first candidate for
future job-scoped foreground/hidden validation; do not add a permanent offscreen
visible window or silent-media trick as the production fix. The all-settings
combination did not demonstrate a consistently better diagnostic result.
The **≥95% foreground gate is UNVERIFIED**, and the earlier unlocked 28–41%
failures are not overturned. Media/audio assertion tricks and V8 throughput are
unmeasured. A short locked run cannot certify long unattended export on every
supported OS. Pixel semantics independently require fallback evaluation.

### Hang Stop and OS checks

The exact `_killWebContentProcess` selector exists and returns, but again failed
to terminate a real infinite loop during the 20-second observation window.
The reset-state variant succeeded in three screen-independent runs:

| Run | Stop→termination callback/observation | Stop→replacement ready |
|---|---|---|
| 1 | 1.048 ms | 126.969 ms |
| 2 | 0.754 ms | 135.136 ms |
| 3 | 0.788 ms | 178.206 ms |

Every success confirmed a positive different replacement PID, the old PID gone,
localStorage preserved and native snapshot revision 7 restored. These are new
measurements, not a promise of the earlier 98.648-ms unlocked result. There is no
separate numeric recovery budget in this spike. **PASS synthetic Stop/restart**;
production reactivation, journal recovery and selective gesture sealing remain
unverified.

**Chosen Stop design (D11, N4/N8):** `_killWebContentProcessAndResetState`, with
macOS major ≥14 and runtime selector checks for both it and `_webProcessIdentifier`.
Unsupported Stop emits `compat_hang_stop_unavailable`; do not blindly call the
SPI or pretend the failed named API is a working fallback. Check both actual
termination and replacement readiness; a callback alone precedes process exit.
This is a technical selection inside the spike, not a release approval. The
[WebKit implementation](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKWebView.mm)
distinguishes terminate() from requestTermination(RequestedByClient) in these two
selectors. That difference is a possible explanation, not a demonstrated root
cause on the installed build. The private header does not supply a per-OS
availability guarantee. macOS 14/15/26 and every future OS release require real
loop/restart tests; selector presence and a numeric OS check cannot certify
behavior. Automatic WebKit termination was not retested: its earlier 20-second
failure remains the evidence. The product still prompts Stop/Wait for trusted
extensions; these explicitly requested test kills do not change D11.

### Budget verdict and fallback

| Requirement | S5b result | Decision |
|---|---|---|
| Scheme page→app p95 ≤0.5 ms | 0.12 ms (2,000 ID calls); batched three-call continuation 0.48 ms | Synthetic latency PASS. Real kernel-actor, IME and VoiceOver gates remain open. |
| 1080p paused sync pixels ≤10 ms including canvas fill | Best cold lower bounds 13.74 ms (pattern) / 19.26 ms (noise); cold continuation 117.26 ms; same-stack new image remains blank | FAIL; no eligible WebKit pixel-promotion design selected. |
| EXT-RASTER-1 JS≤1 ms/layer, complete path≤4 ms/frame p95 | IDs 0.48 ms; warm canvas 1.90 ms; actual engine/corpus not linked | UNVERIFIED, not a whole-path pass. Cold promotion fails. |
| ≥95% foreground throughput hidden/minimised/windowless | Policy-only progresses at 120%/133% of a locked initial phase; combined options have hidden ratios 69%/90% | UNVERIFIED without real foreground. Earlier unlocked failure remains unresolved. |
| Explicit Stop/restart | Exact named SPI fails at 20 s; reset-state succeeds 3/3 at 127–178 ms | Select checked reset-state SPI; synthetic recovery PASS, OS matrix open. |

**Fallback verdict: trigger evaluation of the complete trusted realm in V8 with
remote DOM, pending S1; do not approve the WebKit-only architecture for P1.**
N4 forbids changing sync returns, returning blank canvases, dropping pixel users
or forcing an async API migration. N8 requires the native UI to remain independent
of JS waits. There is no observed supported synchronous native-surface import
into the WebKit realm. Engine-resident ids solve traffic for native consumers,
not the web API's cold observable-pixel semantics. In V8, native code can fill
sandbox-allocator-owned buffers on the isolate's worker thread (§3.5.1) without
waiting on a WebKit decoder/event loop; recorder canvas and DOM bridging still
have to preserve the existing API. This describes why the specified fallback
is the next evaluation, **not proof that V8 already meets these budgets**.
The trusted sync-service cluster must move together; silently splitting its
callbacks between independent realms would break live-object semantics.

Unverified: actual pm-render integration and surface/version reclamation; general
recorder/promotion state and context identity; alpha, colour/HDR and canvas edits;
real override and presenter corpus; frozen v1.2.0 renderFrameTo timings and the
stricter 70% threshold; app→page production raster delivery; real IME/VoiceOver;
quiet-window repetitions; true foreground and long unattended-export comparisons;
macOS 14/15/26 and future releases; V8 sandbox, synchronous pixel API implementation,
remote-DOM behavior and fallback throughput. None is waived by the synthetic pass.
No foreground-session work was attempted or requested tonight.

### Reproduction and checks

```sh
# Pinned workspace build; never edits the shared lockfile.
apps/native/spikes/s5/check.sh build
# If another lane's workspace/lock is incomplete, exact S5 sources via /tmp
# manifest, offline cached dependencies, SAME ~/Developer/pm-target only:
S5_ISOLATED=1 apps/native/spikes/s5/check.sh build
S5_ISOLATED=1 apps/native/spikes/s5/check.sh test
S5_HEADLESS=1 python3 apps/native/spikes/s5/run.py s5b-profile s5b-continuation
# One isolated image candidate per host; repeat pattern/noise and bmp/png/tiff
# with scheme/http; stream is the 64-KiB scheme-response BMP variant.
S5_HEADLESS=1 python3 apps/native/spikes/s5/run.py s5b-images-scheme-noise-bmp s5b-images-stream-noise-bmp
S5_HEADLESS=1 python3 apps/native/spikes/s5/run.py lifecycle-s5b-default lifecycle-s5b-policy lifecycle-s5b-activity lifecycle-s5b-combined lifecycle-s5b-combined-offscreen
S5_HEADLESS=1 python3 apps/native/spikes/s5/run.py kill-spi kill-reset
```

Pinned isolated build and **all four Rust tests PASS**; JavaScript syntax checks
PASS. Normal `--locked` build refused the shared lock update; it was not bypassed
or modified. The isolated manifest uses the exact owned sources and cached
versions, not a second Cargo target directory. Disk was checked before/during
builds and remained above 4 GiB. No cargo clean. Final 14 image cases,
continuation and profiled reproduction exited 0; both default/activity-only
lifecycle cases correctly exited 2 at 160 seconds, the other three exited 0.
Exact hang-kill exited 2 and all three reset recoveries exited 0.

The initial monolithic image diagnostic stalled after scheme PNG/ImageBitmap
and reached the 90-second host deadline; a prior image driver also hit its old
35-second external bound. Initial BMP V4 decoding failed and headerless scheme
images were tainted. These attempts are retained as failures/diagnostics outside
Git, not mixed into the final isolated-candidate quantiles. Final image candidates
use decode/fetch deadlines, explicit CORS, conventional BMP and the combined
job-scoped activity settings; completion does not establish which individual
change removed the earlier stall. A final noisy-image smoke run also checks the
added per-sample noise-pixel assertion.

Raw logs and sampling diagnostics remain outside Git. No private plan, screenshot,
corpus, capability URL or operational dump is included. Only S5 sources and this
report change; no production runtime switch, push, release or tag is performed.
