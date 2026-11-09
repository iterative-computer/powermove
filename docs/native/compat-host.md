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
or the shell's regions. If the shell needs a GPUI/native accessibility anchor
between two DOM groups, instantiate separate hosts for those groups, still
connected to the same V8 realm. Do not flatten the required reading order to
save memory. S5c's roughly 47 MB WebContent measurement favors sharing; it does
not certify the provisional 120 MB compat-page/40 MB Store-view or whole-family
budgets. Measure the actual host count and corpus before freezing those budgets.

The implemented slot geometry is window-content points, top-left origin,
translation and rectangular clips. `present` takes the full window viewport at
origin (0, 0); nonzero viewport offsets are rejected. Nonrectangular/affine slots need a compositor
adapter; the API does not accept or silently approximate those transforms.
Panel bodies stay solid; the host adds no glass to work surfaces.

## Library and shell contract

Every host is one dedicated WebKit document running the production
`PMRemoteDOM.PageApplier` bundle plus `REMOTE_DOM_SHELL`. `new_remote_dom`
builds a trusted or legacy-Store page; `new_store_remote_dom` derives trust and
the profile-scoped data store from an admitted `StoreSession`. A document owns
one native panel, which contains its realm's contribution roots.

`DomPanelHost` exposes `generation`, `create`, `attach`, `detach`, `destroy`,
`apply` and `focus`. Detaching hides the document; destroying drops its applier
and node registry. Mutation sends are bounded admissions, not paint
acknowledgements. Unknown panels, invalid geometry and stale generations are
rejected.

The shell integration points are:

1. Construct `MacDomHost` on AppKit's main thread. A missing WindowServer session
   returns `compat_gui_session_unavailable`; the V8/CLI lane remains independent.
   `install_below(container, gpui_view)` installs its native wrapper below GPUI.
2. Stage the `PanelSlot` and any `set_regions` while publishing the shell layout.
   Call `present` from the vendor P-LAYER present callback, inside the same
   CATransaction as GPUI. It applies the wrapper frame, WKWebView frame, union
   layer mask and the identical native hit mask. It never calls or waits for
   page JavaScript. `composition_dirty` reports geometry the next present owes.
3. Publish `accessibility_view()` once per document in the shell's reading order.
   It retains the real native root and WebKit remote subtree.
4. Host callbacks must enqueue work, never re-enter a borrowed host.

## Channels and Store policy

Page→app messages use synchronous XHR through `WKURLSchemeHandler`, the channel
selected by S5's 0.10–0.12 ms p95 evidence. Each host gets a fresh 256-bit
OS-random capability. Only the exact scheme/authority/path is accepted;
capabilities are never printed or included in Debug output. Messages are limited
to 1 MiB. The outbound queue is capped at 256 entries/8 MiB with explicit
`Backpressure`, never eviction.

`call?q=<encoded JSON>` carries `{generation,panel,kind,payload}` with kinds
`ready` (`{origin}`), `remote_dom` feedback, `canvas_applied`, `fonts_loaded`
and `failure`; replies are `{ok,value}` or `{ok:false,error}`.
`events?q=<last applied seq>` is the one held asynchronous poll. Responses carry
`{generation, seq, events}` with `create`, `slot` (null means detached),
`destroy`, `batch`, `canvas`, `fonts` and `focus`. The next poll acknowledges
only after the page applied the previous batch; unacknowledged data is replayed.
`canvas?q=<id>` and `font?q=<key>` serve native bitmap and face bytes. There is
no hot-path `evaluateJavaScript`, HTTP listener or WebSocket server.

`SyncHandler` runs on the main thread: `event` records host events and
`remote_dom` is a bounded nonblocking mailbox send to the routing worker.

Store rendering documents are loaded with an opaque origin, checked at startup.
They cannot access origin storage. CSP permits only the nonce-bearing bundled
applier to execute. Network permission is derived from the host-owned manifest:
rendered images/styles/fonts/media may use HTTPS only when `network` is granted;
ungranted Store views remain local data/blob only. JS fetch/WebSocket live in
V8 and use permission-enforced kernel calls; page connect-src allows only the
host scheme and local data/blob. Every external/file/data document navigation,
iframe, download, file upload, browser media/motion permission and JavaScript
alert/confirm is blocked. A revision or grant change must retire the rendering
host and re-admit its session, so passive-resource CSP never outlives the grant
snapshot.

Trusted pages present `ui.modal` roots as sheets: the page opens
`about:blank#pm-modal=<frame>` and the host makes a native sheet around that
child web view, at most eight per document, closed on drop. Store pages open no
child windows. Unexpected WebContent termination is reported as
`ProcessTerminated`; crash policy is the app's.

## Background work

`begin_background_job(park)`/`end_background_job` are nestable. They save and
restore `inactiveSchedulingPolicy`, selecting `None` only for the job. When an
editor is unavailable, `park=true` moves the wrapper into an AppKit-visible 1×1
offscreen host with `canHide=false`; ending the final job restores its parent,
sibling order and frame, and closes that host. A later nested job may request
parking. Drop releases any remaining protection. No global settings, permanent
activity assertion or silent-media trick is installed.

## Routing worker

Own `RemoteDomPanel` on the routing worker. Its `enqueue` validates a production
batch through `pm-remote-dom::Router` before admitting an `AdmittedDomBatch` to a
bounded UI queue. AppKit calls `present` on the admitted value and retains it at
the queue front on `Backpressure`. The routing worker calls `receive` to
validate page feedback and admit it to the realm-owner queue. Queue admission is
not a paint acknowledgement. A stopped generation requires a new router and
document.

## Validation

From the repository root:

```sh
apps/native/crates/compat-host/scripts/check.sh test
```

It runs the unit tests, builds the page bundle and runs `examples/offscreen-probe`
against it: opaque origin and storage denial, distinct Store data stores and
content processes, CSP script rejection, navigation and popup denial,
detach/reattach/destroy, and nested background parking. It needs a logged-in
WindowServer session (a locked one works) and claims nothing on screen.
`examples/remote-dom-smoke` renders and inspects a production batch in a window;
`compat-runner`'s `webkit_join` test runs a realm's panels, events, canvases,
modal sheets and a Store page through real WebKit.
