# Powermove GPUI-CE fork

Upstream: https://github.com/gpui-ce/gpui-ce
Pinned commit: `bc577476a0eb55173ef985585cfb487f75ab1cab`.
License: Apache-2.0, retained in LICENSE.md and crate license/notice files.
Shallow-cloned with `git clone --depth 1`, then removed the nested .git.
Omitted root assets, examples, benches, tooling, CI and upstream lockfile.
Only the macOS Metal dependency closure is retained (14 crates). WGPU, other
platforms, fonts/example packages, tests and their dev-dependency edges are omitted.
Linux feature names remain inert for the core test-support manifest; this fork is macOS-only.
The upstream Rust-authored shaders are retained, rather than replacing their pipeline.

Powermove modifications are described below as they are introduced. Upstream source
notices are preserved. No patch has been submitted upstream yet.

References inspected: Frame's excluded vendor workspace/path overrides
(https://github.com/66HEX/frame/blob/master/Cargo.toml), and
https://github.com/luciascarlet/gpui-glass-demo. The latter uses custom WGPU glass;
we use AppKit materials per the native composition contract, with no shader code copied.

Pruned unused crates: gpui_ce_fonts, gpui_elements, gpui_linux, gpui_path, gpui_platform, gpui_tokio, gpui_web, gpui_wgpu, gpui_windows, gpui_zed_util.

## Powermove patches (S2)

Each modified source carries a Powermove notice. The native workspace uses direct
path dependencies and its own lockfile/toolchain. These are additive Metal APIs,
not WGPU emulation. All upstream intent below is proposed, not submitted.

| Patch | What and why | Status / upstream intent |
|---|---|---|
| P-DEVICE | `MacPlatform::new_with_metal_device`, `MetalRenderer::new_with_device` and `new_headless_with_device` retain the app's device. Otherwise preserve upstream enumeration. Atlas, queue and window layer use that same device. Reject imported texture/event device mismatches. | Implemented, builds and headless imports pass. Propose explicit device injection rather than global device state. |
| P-HEADLESS | Expose existing Metal capture/render APIs and `PlatformHeadlessRenderer` in ordinary builds. Autorelease pools bound capture/submission temporaries; unified-memory captures use Shared storage (Managed on discrete GPUs). | Implemented, real GPU captures pass without an unlocked desktop. Full GPUI test app context remains test-gated; AppKit/WebKit cannot be captured by this renderer. Propose production capture feature/API independent of test-support. |
| P-TEXTURE | `NativeTexture` + `Window::paint_texture`; BGRA8Unorm/RGBA16Float, premultiplied SDR sRGB, top-down. Use upstream surface shader's RGB branch and premultiplied ONE / ONE_MINUS_SRC_ALPHA pipeline. Encode MTLEvent waits before render encoders. Deduplicate source fences, retain textures/events/pool leases per submission, notify after completion (including failure). Cached scenes keep their own leases. | Implemented; pixel, delayed signal, repeat-submission, clip and retirement tests pass. Propose typed native texture source with completion lease. No CPU pixel upload/readback in consumption; test producer/readback only in verify. |
| P-LAYER | Main-thread RAII registration invokes a host immediately before GPUI draw/present. Explicit CATransaction and transactional drawable present enclose slot frame changes. Spike hosts a below CAMetalLayer, masked WKWebView, above native field and native glass transport. | Prototype; actual resize/display behavior unverified. Propose frame-scoped composition publication hook, replacing a process-global native overlay. |
| P-HIT | GPUIView hitTest converts AppKit coordinates into top-left points and declines transparent regions. Above siblings and child windows win through AppKit. Host's native SlotView applies matching shape mask/hit clip. | Prototype; deterministic ordering/capture/clip tests pass, AppKit drag tests unverified. Propose host-provided frame hit/occlusion data with ordinary GPUI behavior when unregistered. |
| P-TEXT | Actual NSTextField native slot and a menu search field; AppKit owns responder and composition. | Partial; no full text slot API, multiline field, inFields key resolver, IME/Services test. Upstream intent: native-input ownership integrated with composition slots. |
| P-A11Y | `composition::publish_accessibility` supplies the committed reading order. Ivar-free wrappers on AccessKit's content view and root merge its GPUI nodes with unignored native slot elements, preserving both parent chains. GPUIView forwards AX hit testing; window/panel focus forwards to the content adapter; GPUI focus reclaims the native first responder. | Implemented in S2b. External AX order, parents, frames, WebKit inner field, focus transitions, hit tests and native actions pass with the session locked. Windowless NSTextField/NSTextView/NSButton reorder, removal and teardown probes pass. Spoken VoiceOver traversal remains unverified. Upstream intent: an adapter-supported foreign-child/graft API; this pin has no such API, so the documented AppKit fallback is isolated in the macOS backend. Not submitted upstream. |
| P-A11Y-PRESS | `Div`: a node with click handlers answers an assistive Press (AccessKit `Click`) by calling those handlers with a keyboard `ClickEvent`, as Return does. Upstream's fallback clicked the pointer at the node's centre, which a child with its own handler took (a timeline row's name label swallowed the row's selection). | Implemented; on-screen proof with cua-driver on a timeline row. Upstream intent: the documented on_click → AX Click handler. |
| P-A11Y-SYNTHETIC | `A11ySubtreeBuilder::push_child_at` gives a synthetic child its window bounds: the accessibility frame and the entry the built-in `Click` uses, so an assistive Press clicks its centre as it does an element's. The timeline's clips are synthetic children of the tracks list. | Implemented; headless press test in `app/src/timeline_tests.rs`. Upstream intent: bounds for synthetic children. |
| P-A11Y-CACHE | A cached view (or deferred draw) whose prepaint is reused replays last frame's accessibility nodes under its parent (the frame's node log records each node's depth; the prepaint range records where its nodes start), with their bounds, focus registrations, focus and action listeners, instead of rendering again. The first frame after (re)activation renders every view. Updates to the adapter carry only nodes that changed (none when nothing did); the whole tree is kept for replay and `a11y_tree()`. | Implemented; headless test `crates/shell/tests/a11y_cache.rs` (reuse, focus, press, incremental updates applied give the whole tree). Upstream intent: replay a11y nodes with reused prepaint; incremental tree updates. |
| P-FEEL | Opt-in input-to-photon probe (`PM_FEEL_TRACE=<file>`, `gpui_apple::feel`): AppKit mouse/scroll/key events and app-dispatched input note their timestamp; the next drawable carries the oldest one and its presented handler appends `{ui, path, input, frame, draw, target, presented}` (host seconds). Off without the variable: no handler, file or clock reads. On, a writer thread opens and appends the file (main never touches it; a full queue drops lines). | Implemented; measured by `crates/app/tools/feel.py`. Diagnostic only, not proposed upstream. |
| P-SHEET | `WindowOptions::parent` names the window a `WindowKind::Dialog` attaches to as a sheet (resolved to its `NSWindow` through the parent's window handle); without it, the main window as before. A background app has no main window, and with several projects open the main one isn't always the one that asked. | Implemented; on-screen proof: the Export sheet attaches to its editor window in a background launch. Upstream intent: explicit sheet parent. |
| P-CHILD | Every way a window is shown (GPUI's open, activate and set_visible, tab joins, the app's own raises) goes through `child_window::present`. Child windows (pop-ups, floating windows, the palette, the field menu, the colour panel; marked by hiding on deactivation) sit at the normal level as a child of the window that asked (`WindowOptions::parent`, else the main window), never on every Space, and show only while the app is active: asked for while it's inactive, one waits and shows on activation, unless it or its parent closed or hid meanwhile. Other windows come forward only while the app is active and otherwise appear behind other apps. Attaching resets a child's hiding and Spaces, so `attach` reapplies the policy. | Implemented; headless tests `crates/shell/tests/child_windows.rs` (AppKit window properties, attach and reparent, the inactive-app queue, no direct ordering in the creation sources) and `child_windows_gpui.rs` (GPUI's real open and activate paths with activation prohibited: a PopUp stays off screen, attached, and never shows after its parent closed). Upstream intent: windows owned by their window; nothing from an inactive app over other apps. |
| P-120 | On macOS 14+, `CAMetalDisplayLink` paces the Metal renderer and supplies one retained drawable per callback. Reserve half a refresh before its submission deadline; reject stale updates without draining GPUI demand. Two-frame GPU render-ahead; commit then call `present()` before the CPU deadline, without `nextDrawable` or `waitUntilScheduled` on stable frames. Geometry changes, first publication, untracked hosts and AppKit synchronous paints remain transactional. Release unused callback drawables; invalidate the link on window teardown. The older CVDisplayLink fallback retains continuous-frame coupling. `PM_GPUI_CV_DISPLAY_LINK=1` / `PM_GPUI_CA_DISPLAY_LINK=1` select diagnostic alternatives. Native AppKit tab-bar accessories are no longer hidden/collapsed. | CPU headroom and drawable-wait improvement measured; **zero-miss display gate still FAIL**. Contract/deadline tests in `spikes/s2/src/p120.rs`, grouped/cache pixel comparison in `--verify`, native single-window tab-bar probe in `--verify-tab-bar`. See the 2026-11-01 pacing section in `docs/native/s2-gpui-ce.md`. Upstream intent: compositor-paced frame source with callback-owned drawable and geometry-scoped transactions. |

### Ownership and colour

`NativeTexture::from_borrowed_raw` bridges objc2-metal to metal 0.33: non-null,
live +0 pointers are retained to +1 exactly once, then released by owned wrappers.
The caller pins its wgpu HAL resources until that retain completes. The producer
signals the supplied value from its queue, and must not rewrite/recycle the source
while any scene or command-buffer lease remains. Completion is sampling complete,
not scanout, and callbacks must be short/nonblocking. Removing UI references is not
permission to recycle. The CPU never waits for the producer event.

RGBA16F support is **not** HDR presentation. The engine must convert extended-linear
content into display-referred SDR sRGB for this BGRA8 GPUI target. S3 owns EDR output
in the below-slot layer. The API cannot infer the encoding from a pixel format;
incorrect producer tagging is still a contract violation. Device owners must retire
all cached sources on device loss; failed consuming submissions also retire their
sources. Real device loss and the wgpu-30 bridge remain integration tests for S3.

### Composition limitations

S2's host paints a synthetic fixed layout, not a production docking manager.
It publishes ordered slots, masks/transforms, interactive and occluding regions,
compat overlay rectangles and accessibility order. Affine inverse/clip math is
covered in CPU tests; native placement currently supports translation only.
Overlay geometry is a fixed synthetic DOM fixture, not an asynchronous S5 channel.
WKWebView is clipped to panel + overlay union, never composited full-window.
Native pointer capture is delegated to AppKit; deterministic model capture is
separately tested. S2b verifies the foreign-child tree and programmatic focus/actions
without the screen. Native drag/clip coverage, resize lag, spoken VoiceOver,
keyboard/IME handoff and glass appearance still require live verification.

The glass implementation copies NSToolbar/material/control placement from
`apps/mac/crates/glass`, with pre-26 and Reduce Transparency material fallback.
Solid GPUI panels and viewer surround never use glass. Refraction above GPUI is
only the native-content transport and child field menu. In-window GPUI-child glass
would be below-band; it must not be implemented with an above-slot native bar.
The pooled menu uses the public CASpringAnimation duration/bounce initializer
(0.32/0.15), scale 0.96 → 1, non-bouncing 0.12 exit and 0.15 Reduce Motion fade.
It resumes from current presentation values; velocity continuity, keyboard/typeahead,
glider and all transient kinds remain incomplete. See docs/native/s2-gpui-ce.md.

### P-A11Y implementation and pin assumptions (S2b, 2026-11-01)

Shell accessibility follow-up (2026-11-01): `composition_accessibility::focus`
also forwards a native first responder outside the published slots (AppKit's
standard toolbar). Previously its ring was visible but the content adapter
returned no AX focused element because GPUI did not own focus. It uses the
responder's real focused element or AppKit's unignored ancestor, without copying
native controls into AccessKit. `apps/native/scripts/shell-ax.swift` checks the
external AX focused element alongside the keyboard walkthrough. The fallback
only considers native responders outside the content adapter, avoiding a call
back into that same adapter when a popup restores the initial responder.


The pinned `accesskit_macos` 0.26.3 `SubclassingAdapter` exposes only AccessKit
platform nodes. It cannot accept an NSView child. We retain it, then wrap its
**actual window content view**, which is separate from GPUIView. The adapter's
single `Role::Window` root remains in the tree. A second, per-instance wrapper on
that root merges `accessibilityChildren` and `accessibilityChildrenInNavigationOrder`.
This preserves GPUI's AXParent relationships rather than flattening its root.
Both wrappers add no ivars; weak, main-thread registries locate their state.
Teardown restores the original classes before the upstream adapter is dropped.

`AccessibilityItem::Gpui(AXIdentifier)` selects an exposed direct root child;
`Native(NSView)` inserts that slot's unignored AppKit accessibility elements;
`RemainingGpui` preserves all other GPUI children in their original order. A native
control may expose its cell rather than its NSView. Using AppKit's unignored-child
API avoids an AXUnknown wrapper and duplicate identifiers. Missing anchors cannot
drop GPUI children; duplicate object identities are emitted once. Nested GPUI
subtrees stay intact: a host that needs an insertion point must publish a direct,
exposed root child with a stable identifier. Z order and reading order are distinct.

The host publishes the retained order in its present callback, with slot frames.
AX getters read that snapshot without entering the host or taking GPUI's window
mutex. Native elements keep their own frames, actions and text interfaces; their
AXParent is the retained AccessKit root. Removal/unregister clears overridden
parents. Structural changes post an AppKit layout notification asynchronously,
after the draw lock is released. Ordinary pointer-hit callbacks avoid copying the
accessibility snapshot.

The content-view AX hit test uses AppKit's ordinary composition hit test, including
slot clipping, then delegates to the winning native slot. GPUIView forwards its AX
hit query to the content adapter. GPUIWindow/GPUIPanel use AccessKit's standard
focus forwarder. Native focus queries use WebKit's actual responder for a remote
DOM element and the owning NSControl for its shared field editor. An AccessKit
Focus action makes GPUIView first responder before updating GPUI's logical focus. GPUIWindow/GPUIPanel also observe
successful `makeFirstResponder:` changes and queue AccessKit host-focus updates
after the native action unwinds. Host focus requires GPUIView itself to own the
responder; a native slot in the same key window must clear the previous GPUI
node's AXFocused flag. The adapter is shared through a main-thread RefCell so this
synchronization never enters the GPUI window mutex or re-borrows AccessKit's tree
during an AX action. The external probe verifies losing and regaining AXFocused
as well as the application's focused element.

Verification lives in `spikes/s2/scripts/check-ax.sh`. Its external Swift walker
uses AXUIElementCreateApplication; its independent AppKit-only probe compiles these
exact vendor sources and never shows a window. All artifacts stay outside Git.
The root shape, upstream subclass implementation and native control behavior are
pin-specific assumptions to recheck at each GPUI/AccessKit rebase. No system-wide
method swizzle, WebKit DOM mirror or new AccessKit dependency fork was introduced.

### P-TEXT/P-120 pass (2026-11-01)

This is an **implementation / screen-free test pass**, not a 120-Hz or S2 gate pass.
The session locked before the after-patch display and OS IME checks; those are open.

- **P-TEXT:** `gpui_macos::native_text::NativeTextSlot` owns real `NSTextField`
  and scrollable plain `NSTextView` editors. Primary/multiline fields stay native;
  dense text/number fields become native only while editing. Geometry and inherited
  rectangular clips are staged from GPUI paint, then applied in the composition's
  `CATransaction`. No custom `NSTextInputClient`, key injection or marked-string
  conversion is installed. AppKit keeps its field editor, responder chain, undo,
  selection, marked text, contextual Services, dictation and emoji input. Spell
  check/correction is enabled for text (disabled for numbers). Writing Tools uses
  the system default behavior where the selector exists; actual system availability
  remains an OS/account/hardware condition, not an emulated feature.
- **P-TEXT / AX:** live native editors join the retained composition reading order;
  an explicit `accessibility_item()` lets the host anchor one at a panel position.
  Otherwise managed text editors are appended, with identity deduplication.
  Visibility changes/removal notify the existing P-A11Y adapter; drop removes the
  native subview and parent override. Placement currently supports rectangular
  clipping and translation, matching the existing S2 host limitation.
- **P-TEXT / shell:** `pm_shell::text::TextField` renders dense values in GPUI and
  overlays an identically sized native slot while editing. Number validation runs
  at commit, allowing incomplete/marked strings during editing; invalid values or
  Escape restore the previous value. Change/commit/cancel events wake GPUI through
  a channel. Primary and multiline kinds use the same component API. The shell and
  popup key monitors return marked-text events directly to AppKit. Native Edit
  menu roles expose spelling, Writing Tools and Emoji & Symbols; Services retains
  its standard application menu. No dictation permission is requested by tests.
- **P-120:** retain the existing per-display CoreVideo link and its safe immortal
  registry. Starting an already subscribed display is now a no-op; a display move
  changes subscriptions, and occlusion/no-screen stops them. No fixed 60-Hz timer
  or unsafe display-link teardown was added. ProMotion pacing still needs an
  unlocked run. `PM_FRAME_PACING=1` records the **GPUI layer's** drawable timestamps;
  zero timestamps are unknown, never successful presentations. CPU phase timing
  remains opt-in through `ZED_MEASUREMENTS=1`.
- **P-120 / composition:** `MacWindow::draw` releases the window lock before the
  slot transaction commits. The commit runs AppKit's display cycle, whose cursor
  rect reset takes the same lock; holding it deadlocked the main thread whenever
  the viewer slot moved.
- **P-120 / sync paint:** `display_layer` and `window_did_change_key_status`
  invalidate the window's display link before their synchronous `nextDrawable`
  paint (`release_display_link_for_sync_present`; Core Animation raised on any
  window resize or key change, even with the link paused); the next start
  attaches a fresh link. While a link owns the layer, `MetalRenderer::draw`
  without a link-supplied drawable returns `false` and the window repaints on
  the link's next update, never calling `nextDrawable` (HANDOFF P-120 rule).
- **P-120 / headroom candidate:** S2 caches its unchanged native viewer pattern and
  clip masks, and gives the 1,000 static texture tiles a cached GPUI entity. The
  fresh 60-Hz producer is separate; texture leases/fences remain intact. This uses
  GPUI's existing invalidation-aware `.cached(...)` API; it does not introduce a
  second scene cache. An explicit `S2_BASELINE=1` disables these fixture caches for
  A/B checks. A real 5,000-row `uniform_list` scroll workload advances seven rows
  on every requested display frame (`S2_TIMELINE=1`), bounded to 7,400 frames.

Tests: the native AppKit probe checks multiline/UTF-16 marked text and commit,
spell/undo configuration, clipping, equal dense/native geometry and teardown,
then reruns the original foreign-child AX bridge probes. A real headless GPUI
regression test confirms twelve parent invalidations reuse an unchanged child,
identical rendered pixels, and a child notification invalidates its cache.
Reproduction and open gates: `docs/native/s2-gpui-ce.md`. Upstream intent is an
optional native text-slot facility and idempotent frame-source start; no submission
has been made. No measurements from a locked or occluded session certify pacing.
- **P-TEXT / canvas kind:** `TextKind::Canvas` is an invisible `NSTextView`
  (alpha 0, inside a hit-test-transparent host) that owns typing, IME, dictation,
  emoji and spelling while the app draws the text from its engine. It reports
  `Selection`, editing chords (`Key`) and `Blurred` instead of committing, keeps
  its last placement when a frame skips its host's paint (so it stays first
  responder), and shows the system insertion point (`NSTextInsertionIndicator`)
  at a caret rectangle the host stages.
- **Development review frames:** debug builds with `PM_NATIVE_DRAW_OCCLUDED`
  set keep an occluded window's display link running, so background lane checks
  see live frames without raising a window. Release builds always pause it.
