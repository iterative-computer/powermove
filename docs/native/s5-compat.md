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
`spikes/*`. No workspace-member edit is needed. The existing empty `crates/*`
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
used to deliver these calls.

## 1. Channels under scrub load

Each isolated candidate used 10 warmups and **2,000 measured calls** with
cross-origin isolation enabled (observed clock increment **0.020 ms**). Scrub
work p95 was 1.053/1.047/1.085 ms in HTTP/scheme/prompt runs. Actual timer interval
p95 was 9.652/10.251/9.823 ms: requested 120 Hz, with measurable scheduling jitter.

| Candidate | p50 / p95 / p99 (ms) | Result / decision |
|---|---|---|
| Sync loopback HTTP, same-origin XHR | 0.400 / **0.660** / 1.040 | **FAIL** observed p95 >0.5 |
| WKURLSchemeHandler on main | 0.080 / **0.100** / 1.060 | Latency PASS; first provisional candidate by §3.5.2 ordering |
| Text-input prompt delegate on main | 0.060 / **0.060** / 0.080 | Latency PASS; later candidate, not preferred by rule |
| Page-held long-poll, 400 calls | 1.271 / **2.502** / 6.801 | Ack upper bound; slower observed path |
| Off-main loopback WebSocket, 400 calls | 0.271 / **0.575** / 0.717 | Provisional app→page choice; no hot-path eval |

**No production channel is approved.** Scheme/prompt measure immediate synthetic
native replies, not a completed kernel-actor integration. IME/real WebContent AX
and supported-OS evidence are missing. A quiet-window repeat must confirm HTTP's
failure; this shared-machine observation does not establish an intrinsic limit.
App→page numbers include JS and return-trip overhead. WebSocket is chosen
provisionally because its observed delivery+ack bound is lower; EXT-RASTER-1
still needs a real override workload.

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

Optional stub not yet implemented. Real write-barrier, retained references,
observer order, native edit races, external undo, lease folding, selective cancel,
owner-kill recovery/journal, and frozen-reference traces remain open. This pass
also does not close extraction, inspector/cluster switch, recorder promotion,
Store-process isolation, virtual time, presenter or 39-extension memory gates.

## Current failures for Jude before P1 sizing

- HTTP channel p95 0.660 ms exceeds 0.5 ms in this shared-machine run.
- Sync HTTP pixels and nested-capture canvas path greatly exceed 10 ms; IDs
  preserve native buffers but cannot replace observable canvas/pixel APIs.
- SPI activity diagnostic ratios are 74.0%/69.2%; offscreen-only hidden job does
  not complete before 160.055 s. There is no true foreground baseline while locked.
- Exact `_killWebContentProcess` and automatic WebKit termination did not stop
  the real loop in 20 s. Reset-state SPI works in one probe; requires a decision.
- Full IME/VoiceOver, kernel-actor scheme round trip, quiet-window repeats,
  frozen-reference paired traces/timings, supported OS runs and the V8+remote-DOM
  fallback remain unverified. No compatibility exception or P1 approval follows.
