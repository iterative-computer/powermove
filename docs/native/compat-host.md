# Compat WebKit host

`apps/native/crates/compat-host` supplies the Mac-only `pm-compat-host` library.
It renders DOM panels for the V8-owned v1–3 realm. Extension logic, synchronous
service objects, recorder canvases and authoritative project state belong to the
V8/kernel/engine lanes. This follows orchestrator decision 5 in
[p0-decisions.md](p0-decisions.md), superseding the older WebKit-owned realm.
No spike, shell or vendor source is changed by this crate.

## Placement decision and rule

**Default: one shared rendering page per trusted window**, with a logical root
per panel. Create it on demand. Store extensions each get a separate host and a
separate `WKWebsiteDataStore`; they never join a trusted page or another Store
extension's page. `StoreDataStores` is an app-owned, main-thread registry. Its
nonpersistent data stores survive host replacement while the registry lives;
durable extension storage remains kernel-owned.

**Rule:** share a page only where its panels form one below-band composition and
accessibility group. Clip the shared page to the union of visible panel clips
and reported overlays. If the shell needs a GPUI/native accessibility anchor
between two DOM groups, instantiate separate hosts for those groups, still
connected to the same V8 realm. Do not flatten the required reading order to
save memory. S5c's roughly 47 MB WebContent measurement favors sharing; it does
not certify the provisional 120 MB compat-page/40 MB Store-view or whole-family
budgets. Measure the actual host count and corpus before freezing those budgets.

The implemented slot geometry is window-content points, top-left origin,
translation and rectangular clips. `present` takes the full window viewport at
origin (0, 0); nonzero viewport offsets are rejected. Nonrectangular/affine slots need a compositor
adapter; the API does not accept or silently approximate those transforms.
Panel bodies stay solid; the fixture adds no glass to work surfaces.

## Library and shell contract

`DomPanelHost` exposes `generation`, `create`, `attach`, `detach`, `destroy`,
`apply`, `send_event`, `focus` and `composition`. Detaching hides a live DOM root
and clears its overlay holes; destroying removes its root and node registry.
Mutation/event sends are bounded admissions, not paint acknowledgements.
Unknown panels, invalid geometry and stale mutation generations are rejected.

The shell integration points are:

1. Construct `MacDomHost` on AppKit's main thread. A missing WindowServer session
   returns `compat_gui_session_unavailable`; the V8/CLI lane remains independent. Retain it through slot teardown.
   `install_below(container, gpui_view)` installs its native wrapper below GPUI.
2. Stage changed `PanelSlot`s while publishing the shell layout. Call `present`
   from the vendor P-LAYER present callback, inside the same CATransaction as
   GPUI. It applies the wrapper frame, WKWebView frame, union layer mask and the
   identical native hit mask. It never calls or waits for page JavaScript.
3. Use `CompositionSnapshot::accepts_gpui` in the P-HIT callback. The vendor
   callback already supplies **top-left** points. Supply GPUI's interactive AND
   visually occluding rectangles. Child windows and above-slots keep AppKit
   priority. Native pressed-view drag capture remains AppKit-owned.
4. On `HostEvent::CompositionChanged`, schedule a shell frame. Overlay bounds
   come from real DOM `getBoundingClientRect` calls, are capped at 128 per panel,
   validated and clamped to the viewport. Mutation/resize/scroll observations
   refresh them. Arbitrary CSS transform animations need the full bridge's
   overlay observer; this minimal applier does not sample animations each frame.
5. With feature `gpui`, merge `accessibility_item()` into
   `gpui_macos::composition::publish_accessibility` alongside the shell's GPUI
   anchors. The item retains the real native root and WebKit remote subtree.
   Publish each shared page once. DOM roots are ordered by `reading_order`.
   Unregister composition before dropping hosts. Spoken VoiceOver is unverified.
6. Before resolving a native key/menu equivalent, yield while
   `owns_first_responder()` is true. DOM panel handlers and the compat keymap run
   first. The bundled applier forwards unconsumed keys after bubble handlers,
   preserving fields, composition, selection Copy and Enter-on-button defaults.
   Feed `HostEvent::Key` into the shell resolver with `compat=false`, retaining
   order and field/repeat/key-up flags. Never replay the same key into WebKit.
   The shell still owns asynchronous may-decline resolution, bypass chords and
   menu replay. The existing shell monitor must wire this seam before rollout.
7. Call `tick` outside the GPUI draw lock while recovery or popouts are active.
   Host callbacks must enqueue work, never re-enter a borrowed host.

`examples/shell-smoke.rs` compiles these hooks into an actual GPUI window.
The crate does not modify the parallel shell lane's dependencies or key monitor.
Resize endpoints use the shared geometry; ≤1-frame DOM reflow and snapshot
stretching are not certified here.

## Channels, permission checks and Store policy

Page→app calls use synchronous XHR through `WKURLSchemeHandler`, the channel
selected by S5's 0.10–0.12 ms p95 evidence. This crate makes no new latency claim.
Each host generation gets a fresh 256-bit OS-random capability. Only the exact
scheme/authority/path is accepted; capabilities are never printed or included in
Debug output. Arguments and single mutations are limited to 1 MiB. The outbound
queue is capped at 256 entries/8 MiB with explicit `Backpressure`, never eviction.

`SyncHandler::call` receives the host-owned trust/extension identity, generation,
panel and method. Store methods must appear in the app-owned method→permission
map and the manifest's granted permissions. Unknown or ungranted methods fail
before dispatch. The kernel must independently enforce resource permissions.
**Handlers run on the main thread:** return a bounded native/cached answer or
enqueue work; never wait for V8, the kernel actor, GPU completion or page JS.

App→page uses one held asynchronous scheme request. Responses carry
`{generation, seq, events}`. The next poll acknowledges only after applying the
previous batch. Unacknowledged data can be replayed; stopped tasks are removed
before any further callback. There is no hot-path `evaluateJavaScript`, HTTP
listener or WebSocket server. Tests use eval solely for inspection/fault injection.

Store rendering documents are loaded with an opaque origin, checked at startup.
They cannot access origin storage. CSP permits only the nonce-bearing bundled
applier to execute. Network permission is derived from the host-owned manifest:
rendered images/styles/fonts/media may use HTTPS only when `network` is granted;
ungranted Store views remain local data/blob only. JS fetch/WebSocket live in
V8 and use permission-enforced kernel calls; page connect-src allows only the
host scheme and local data/blob. Every external/file/data document navigation,
iframe, download, file upload, browser media/motion permission and JavaScript
alert/confirm is blocked. Native file/media operations go through the kernel's
permission workflow. This is a rendering host, not a Store runtime-WebView
fallback or a complete port of sandbox runtime globals.

Trusted popouts are created with WebKit's supplied configuration in
`createWebViewWithConfiguration`, preserving opener references and separate
same-origin documents. They are parented child NSWindows, limited to eight live
children and cleaned up on close/reset/drop. Store popouts are denied. The
fixture demonstrates an opener-held child object; complete remote-DOM popout
node/event/closure semantics belong to the bridge lane.

## Background work and recovery

`begin_background_job(park)`/`end_background_job` are nestable. They save and
restore `inactiveSchedulingPolicy`, selecting `None` only for the job. When an
editor is unavailable, `park=true` moves the wrapper into an AppKit-visible 1×1
offscreen host with `canHide=false`; ending the final job restores its parent,
sibling order and frame, and closes that host. A later nested job may request
parking. Drop releases any remaining protection. No global settings, permanent
activity assertion or silent-media trick is installed. These mechanics do not
establish ≥95% background export throughput: V8/engine own export work, and the
S5b 90–106% diagnostics are not a foreground throughput certification.

`Watchdog` consumes **JS execution time supplied by the runtime/bridge**, excluding
native waits. It emits a 2 s notice and an 8 s trusted Stop/Wait decision; Store
hangs request automatic Stop. Wait allows another 8 s of JS execution. The host
does not infer JS time from a wall-clock heartbeat or silently stop trusted code.

After approved Stop, `stop_and_recover` checks macOS ≥14 and both reset/PID SPIs.
`SyncHandler::prepare_reset` runs first: the kernel/bridge must seal acknowledged
gestures, revoke old registrations and cancel old generation callbacks. An error
aborts Stop. The host rotates the capability/generation, clears old polls and
holes, closes popouts and calls `_killWebContentProcessAndResetState`. It reloads
the bundled renderer and recreates logical roots/slots. `Ready` asks the bridge
for an authoritative fresh DOM snapshot. `Recovered` requires a different positive
PID, replacement-page readiness **and old PID exit**, with a 20 s failure deadline.
Unsupported OS/selectors return `compat_hang_stop_unavailable`; the failed named
kill SPI is never used as a pretend fallback. Unexpected WebContent termination
is reported to the shell; automatic crash policy remains an app responsibility.

## Remote-DOM lane handoff

`MacDomHost::new_remote_dom` now accepts the production `PageApplier` browser
bundle and installs `REMOTE_DOM_SHELL` in a dedicated realm document. It permits
one native panel, containing that realm's contribution roots. The fixture
constructor and protocol below remain available for the earlier host probes.

Own `RemoteDomPanel` on the routing worker. Its `enqueue` validates a production
batch through `pm-remote-dom::Router` before admitting an `AdmittedDomBatch` to a
bounded UI queue. AppKit calls `present` on the admitted value and retains it at
the queue front on `Backpressure`. The router does not run on AppKit. Implement
`SyncHandler::remote_dom` as a bounded nonblocking mailbox send; the default
handler refuses feedback. The routing worker calls `receive` to validate page
feedback and admit it to the realm-owner queue. Queue admission is not a paint
acknowledgement. A stopped generation requires a new router and document.

`new_store_remote_dom` derives trust metadata from an admitted `StoreSession`
and keys the retained nonpersistent data store by its full profile-scoped key.
Store method authority still belongs to the live kernel permission check, and
the confined Store helper/facade remains a separate required integration. A
revision or grant change must retire the rendering host and re-admit its
session, so passive-resource CSP never outlives the grant snapshot.

The complete application integration must:

- Drive the main-thread `DomPanelHost` interface and tag mutations with the current
  generation. Queue backpressure must pause/retry upstream without losing order.
- Use `new_remote_dom` with the host-built production applier, or supply another
  trusted renderer through `new_with_applier`. `PM_HOST_CONFIG` supplies `base`
  and `generation`; `applier.js` is an executable fixture, not a
  claim of arbitrary DOM/Svelte/Canvas/CSS or frozen-reference compatibility.
- Use `call?q=<encoded JSON>` for `{generation,panel,kind,method?,payload}` and
  `events?q=<last applied seq>` for the asynchronous poll. Call replies are
  `{ok,value}` or `{ok:false,error}`. Kinds are `ready`, `call`, `dom`, `key`,
  `overlays`, `failure`, and production `remote_dom` feedback; ready includes
  `{origin:location.origin}`.
- Consume `create`, `slot` (null means detached), `destroy`, `batch`, `event` and
  `focus` messages. Report overlay rectangles in window coordinates, DOM input,
  focus, default-prevention/key order, errors and real paint acknowledgements.
- Preserve V8 node/object identity, synchronous services, callback/default-action
  behavior, node/document IDs in popouts, and snapshot reconstruction on `Ready`.
  Enforce safe renderer mutations; do not inject extension scripts or use eval.
  Supply JS-time watchdog accounting and authoritative reset/gesture sealing.

The fixture batch is an array of `[op,node,...args]`: `create` (tag), `append`
(child node), `text`, `attr`, `value`, `remove`. Node 0 is the panel root. The
fixture forwards input/change/click/focus and keys, marks overlays with
`data-pm-overlay`, and uses `data-pm-popout` for its synthetic popout button.
Its bounded tag/attribute list deliberately rejects script/event attributes;
`data-pm-panel` and `data-pm-node` are reserved. The full bridge replaces this
adapter rather than treating the fixture subset as the v1–3 contract.

## Validation and exact commands

From the repository root, no screen access:

```sh
apps/native/crates/compat-host/scripts/check.sh test
```

The script sources `engine/env.sh`, selects Rust 1.96.1 from `apps/native`, uses
only `~/Developer/pm-target`, checks free disk before/during each command and stops
below 6 GiB. It builds no V8 archive and never cleans shared caches.

| Check | Result on 2026-11-01 |
|---|---|
| Rust unit tests | **PASS 22/22**: generation/auth/routing, permission admission, CSP, replay/backpressure, lifecycle model, overlay clip/hit geometry, reading order and watchdog |
| Real WebKit offscreen probe, locked session | **PASS**: opaque origin/storage denial; distinct Store data-store objects and content PIDs; synchronous RPC/permission denial; applier/input→Rust→DOM; DOM bubble-order/decline forwarding, app→page events and real overlay reports; CSP script rejection; navigation/Store popup denial; detach/reattach/destroy; nested background parking/restoration/teardown |
| Real infinite loop, reset-state recovery | **PASS offscreen**: prepare-reset called, stale generation rejected, replacement ready, different PID and old PID gone. One diagnostic recovery ≈43 ms; no performance gate claimed |
| All-feature/all-target compilation and Clippy (`--no-deps -- -D warnings`) | **PASS**, including GPUI P-LAYER/P-HIT/P-A11Y fixture; no shell monitor integration claim |
| On-screen panel, actual typing, overlay click, trusted popout and visual recovery | **UNVERIFIED**. macOS reported the screen locked; foreground attempts failed active/key checks. Owner instructed keeping it locked and finishing without waiting |
| IME, spoken VoiceOver, light/dark/a11y appearance, resize lag, supported-OS and corpus/memory/throughput budgets | **UNVERIFIED** |

When the owner chooses an unlocked foreground session, run exactly:

```sh
apps/native/crates/compat-host/scripts/check.sh smoke
```

This builds first, refuses a locked session, then acquires the shared GUI lock
using `set -o noclobber; echo compathost > /tmp/pm-shots/gui.lock`, retrying every
30 s while held. The owned test is bounded to 35 s by its driver, 45 s by an
independent watchdog thread (which writes `/tmp/pm-shots/compat-host-stall.txt`,
a stack sample, before exiting 3) and 90 s by the outer guard. Any failure exits
non-zero. It removes its lock on exit and closes its own app. Do not interact with the fixture while
its synthetic keys/clicks run. Look for:

1. A real GPUI window with a solid right-side “Remote DOM panel” inside its slot.
2. `powermove` typed into “Compat name”, with `Value: powermove` underneath.
3. A purple overlay protruding left of the panel; its button changes to
   `Overlay clicked`, proving the native hit hole.
4. A separate child “Compat popout” window with its own input field.
5. A deliberately hung page replaced by the restored panel, while native shell
   rendering continues. The log ends `SMOKE PASS typing overlay popout recovery activity`.

Inspect `/tmp/pm-shots/compat-host-{typing,overlay,popout-parent,popout,recovered}.png`
locally. Captures and raw logs must remain outside Git. A compiled script and an
offscreen DOM event are not an on-screen typing/visual pass. No full S5, remote-DOM
compatibility, P1, release or supported-OS approval is implied.

## On-screen smoke attempt (2026-11-01, ~10:12–10:17)

**UNVERIFIED: no pass is claimed.** `check.sh smoke` ran twice:

1. It built and acquired `compathost`, and logged `SMOKE ready_generation=1`,
   `SMOKE native_foreground active=true key=true visible=true` and
   `SMOKE foreground=true responder=true`. The script's own disk guard then stopped
   it (`compat_disk_or_8_minute_limit`) before typing, the overlay click, popout or
   recovery. Free space fell below 6 GiB because this lane ran a release build at
   the same time; that was a scheduling error, not a host failure. No captures
   were written.
2. After disk was restored (9.7 GiB), the script correctly refused with
   `compat_screen_locked: smoke UNVERIFIED`. The Mac locked at ~10:17.

Only readiness and foreground/key/responder acquisition are observed. The five
visual checks above remain open. Rerun `check.sh smoke` on an unlocked Mac with
≥ 6 GiB free and no concurrent build.

## On-screen smoke rerun (2026-11-01, 10:52–11:02)

**FAIL: the smoke stalls after foreground acquisition, with a blank panel.**
`check.sh smoke` at `0863a9732` built (1 min 17 s, 12 GiB free, no concurrent
build: the shared build lock was held) on an unlocked Mac. It acquired
`compathost` and logged `SMOKE ready_generation=1`,
`SMOKE native_foreground active=true key=true visible=true` and
`SMOKE foreground=true responder=true`. Nothing followed. `shell-smoke` kept
running at ~25% CPU well past its 35 s bound. The outer guard ended it at
8 min (`compat_disk_or_8_minute_limit: owned child stopped`) and the lock was
released. No `compat-host-*.png` captures were written.

A screenshot at 7 min 51 s (`/tmp/pm-onscreen2/compat/hung-window-small.png`,
outside Git) shows the GPUI window, sidebar, timeline strip and the purple
overlay protruding left of the panel slot. The **slot itself is blank**: there
is no "Remote DOM panel" content and no "Compat name" field, and the
right-hand edge of the slot shows the window behind it. Checks 2–5 (typing,
overlay click, popout, recovery) therefore did not run. The bounded 35 s
owned test also failed to stop itself, so the internal deadline needs a look,
not just the panel attach.

## Smoke fix and on-screen pass (2026-11-01, P1 fix lane)

**PASS on screen:** `check.sh smoke` logged `SMOKE PASS typing overlay popout
recovery activity` and exited 0 after ~10 s of fixture time. It ran on an
unlocked Mac under the GUI lock with no concurrent build. Captures are in
`/tmp/pm-p1fix/compat/run3/` (outside Git). The rerun-2 stall (`feat/na-onscreen2`)
had four causes:

1. **Blank panel slot (applier bug).** The page stylesheet makes each panel root
   `position:fixed`, and `layout()` writes its left/top/width/height inline. The
   fixture's `attr 0 style …` replaced the whole `style` attribute, which erased
   that geometry. The root collapsed to page (0,0), behind the opaque GPUI
   chrome, so the slot showed only WebKit's empty background. Any remote content
   that styles its root would do the same. `applier.js` now re-applies the
   host-owned geometry after every batch.
2. **Driver timer stopped (stall with no timeout).** The 35 s check ran on an
   `NSTimer` scheduled only in the default run-loop mode. The rerun-2 hang
   screenshot shows AppKit's autocorrect bubble for the typed word. A correction
   UI or event-tracking loop runs outside the default mode, so the driver (and
   its deadline) stopped while GPUI kept rendering (~25% CPU). The timer now runs
   in the common modes. The fixture input sets `spellcheck`/`autocorrect`/
   `autocapitalize`/`autocomplete` off; the applier's attribute allowlist
   gained these four harmless attributes.
3. **Unenforced and misreported bounds.** A watchdog thread now enforces 45 s
   independently of the main thread, writes a stack sample to
   `/tmp/pm-shots/compat-host-stall.txt` and exits 3. The outer guard for the
   smoke is 90 s (`GUARD_SECONDS`; 8 minutes still for builds/tests). Failures
   used to exit **0**, because GPUI's quit terminates the process before `main`
   could return a status. They now exit 2 directly, with a `failure` capture.
4. **Recovery re-entrancy abort.** `stop_and_recover()` closed the popout window
   synchronously. AppKit made the parent key and GPUI rendered inside that
   callback, and the paint closure re-borrowed the host that the caller still
   held: a `RefCell` panic in a non-unwinding callback. Host methods now close
   popout windows on the next run-loop turn (`close_later`), so no host call
   re-enters the shell.

The smoke fixture also leaves GPUI's chrome transparent over reported overlay
holes, so the protruding overlay is visible as well as clickable.

| Check (on screen, 2026-11-01 ~11:53) | Result |
|---|---|
| Panel in its slot | **PASS**: "Remote DOM panel", "Compat name" field, popout button |
| Real typing | **PASS**: `powermove` typed, `Value: powermove` shown (`typed=powermove`) |
| Overlay click through the native hole | **PASS**: `click node=6`, button reads `Overlay clicked` |
| Trusted popout | **PASS**: separate "Compat popout" child window with its own input (`popout_count=1`) |
| Hung page → recovery | **PASS**: real infinite loop, generation 1 sealed, generation 2 ready and restored panel, shell kept rendering |
| Bound: driver stuck (`PM_COMPAT_SMOKE_FAULT=stuck`) | **PASS**: `SMOKE timeout stage=1`, exit 2 at 35 s |
| Bound: main thread blocked (`PM_COMPAT_SMOKE_FAULT=stall`) | **PASS**: `SMOKE watchdog_timeout`, exit 3 at 45 s, stack sample shows the blocked main thread |
| Offscreen probe / unit tests | **PASS** 7/7 / 22/22 |

Still unverified: IME, spoken VoiceOver, appearance variants, resize lag,
supported-OS matrix and budgets. The synchronous `pmCompatHost.call` XHR
remains the RPC contract. A page blocking on it while AppKit waits on WebKit
was considered as a hang cause, but it was not observed.
