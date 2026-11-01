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
| P-120 | Upstream display link and uniform_list retained; native action channel wakes GPUI without polling. | No new pacing/cache patch. No 120 Hz display/frame-time proof. Propose changes only after measuring actual invalidation and display moves. |

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
Focus action makes GPUIView first responder before updating GPUI's logical focus.

Verification lives in `spikes/s2/scripts/check-ax.sh`. Its external Swift walker
uses AXUIElementCreateApplication; its independent AppKit-only probe compiles these
exact vendor sources and never shows a window. All artifacts stay outside Git.
The root shape, upstream subclass implementation and native control behavior are
pin-specific assumptions to recheck at each GPUI/AccessKit rebase. No system-wide
method swizzle, WebKit DOM mirror or new AccessKit dependency fork was introduced.
