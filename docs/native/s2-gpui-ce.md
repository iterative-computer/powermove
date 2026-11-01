# S2 — GPUI-CE composition spike

**Status: partial implementation; S2 gate is NOT passed.** No framework kill has
been demonstrated, but native composition, accessibility and performance remain
unverified. Do not treat this as P1 approval. No push, release or tag.

## Pin and reproduction

GPUI-CE: `bc577476a0eb55173ef985585cfb487f75ab1cab` from
https://github.com/gpui-ce/gpui-ce. Apache-2.0 licenses/notices retained.
Shallow clone, nested Git metadata removed, pruned to the 14-crate macOS Metal
closure (about 5.3 MiB). Frame's vendor-workspace layout was inspected; the
WGPU glass experiment was read, but its shaders were not copied.

Patch details, ownership and upstream intent:
[POWERMOVE.md](../../apps/native/vendor/gpui-ce/POWERMOVE.md).

```sh
apps/native/scripts/check-s2.sh
```

This sources `engine/env.sh`, uses **only** `~/Developer/pm-target`, runs build,
contract tests and `pm-native-s2-spike --verify`. Run the binary without arguments
for the window demo. `cargo-guard.py` enforces the shared target even when invoked
directly, refuses builds below 2.5 GiB free, and stops only its own Cargo process
group if the threshold is crossed. Do not run Cargo from the repository root:
toolchain selection follows the current directory, not `--manifest-path`.

### Verification limits

The successful full build and real GPU tests below ran with Rust
**1.99.1** selected from the repository root. The required copied native toolchain
is **1.96.1**. Its final Cargo test attempt was stopped by the disk guard before
completion. That direct invocation also accidentally omitted the target env var;
the newly created 402 MiB `apps/native/target` was removed, and the guard now
sets the shared target internally. No `cargo clean` was used. Free space recovered
to about 2.8 GiB. No further full builds were attempted after the stop.

The native 1.96.1 formatter check passes. All seven current contract tests also
pass on 1.96.1 using a standalone rustc test harness, with the small test binary
in the shared target. This avoids rebuilding the GPU dependency graph. The pinned-toolchain **full build/test is
unverified**, not a compiler failure. The latest built window process survived a five-second
launch smoke test without logged errors. This only proves process liveness, not
correct appearance, focus or presentation. The screen was
locked. No unlock was requested and no screen-capture evidence is claimed.

## Decisions and picking rules

| Decision | Rule that picked it |
|---|---|
| Keep the upstream Metal renderer; explicit app-owned device injection, no global mutable GPU device. | §3.1 P-DEVICE; engine picks the shared device. |
| Reuse the existing glass crate's NSToolbar/AppKit layout approach and RGB patch's scene + GPU-submission leases, rather than rebuild those experiments. | Reconciliation reuse requirement; N5 prune/reuse. No apps/mac files changed. |
| Engine pixels stay on GPU; MTLEvent producer wait and consuming-buffer completion govern reuse. | P-TEXTURE, N3 correctness and N8 overhead. |
| RGBA16F input is already converted SDR sRGB for GPUI; the EDR viewer remains a separate below-slot. | §3.1 colour contract / §3.3; format alone cannot prove colour encoding. |
| Publish one frame's ordered slots, local clip, local-to-window transform, GPUI interactive + occluding regions, compat holes and accessibility order. | §3.1.2 atomic composition contract; N3. |
| Use AppKit's sibling ordering and transparent GPUIView hit-test delegation; use native SlotView mask and matching hit clip. | §3.1.2 hit order; pressed AppKit responder keeps native drags. |
| Slot geometry is applied from GPUI's present hook within the same CATransaction; transactional Metal present is enabled. | §3.1.2 resize/animation contract. Actual ≤1-frame behavior still needs measurement. |
| Keep WKWebView full-window internally but clip its layer/hit area to panel + overlay union. | §3.1.2 compat compositing budget. Fixture rectangles are synthetic; S5 owns reporting real DOM overlays. |
| Solid viewer/timeline/sidebar surfaces; glass only in NSToolbar, the native-content transport and pooled child menu. Pre-26 materials and Reduce Transparency solid material fallback. | N9, §3.10, §3.1.2 Glass rule. GPUI-child glass would be below-band; no above glass pretending to refract GPUI children. |
| Use a pre-created field-menu child window and the public CASpringAnimation duration/bounce initializer, 0.32/0.15. Exit opacity is 0.12 without bounce; Reduce Motion is a 0.15 fade. | §3.10 motion tokens; [Apple spring API](https://developer.apple.com/documentation/quartzcore/caspringanimation). Full interrupt velocity and field-menu interaction remain incomplete. |
| Preserve upstream display link/virtual list first; native control channel wakes GPUI without an idle timer. | N8; P-120 changes need actual frame evidence. |
| Stop full builds at the disk threshold and leave the native gate open. | Explicit S2 machine limit supersedes the general §9 80-GB assumption; no fabricated gate result. |

## Patch status

| Patch | Result |
|---|---|
| P-DEVICE | Implemented; window + headless constructors and platform injection. Headless imported textures use supplied device. Different-device hardware unavailable. |
| P-HEADLESS | Implemented; ordinary-build Metal capture and async offscreen submit; real pixels verified. Full app test context remains test-gated. Native AppKit/WebKit capture is not provided by this API. |
| P-TEXTURE | Implemented; BGRA8/RGBA16F, retained raw-pointer shim, producer event, per-consumer completion leases, repeat-scene retention and retirement. Hardware device-loss/wgpu-30 bridge unverified. |
| P-LAYER | Prototype; below CAMetalLayer, clipped WKWebView, above NSTextField + native glass bar, presentation-boundary transaction hook. Native placement supports translations only; arbitrary affine placement is not finished. |
| P-HIT | Prototype; top-left coordinate conversion, GPUI delegation, native clip/hit wrapper and deterministic contract tests. Native drag/capture not exercised. |
| P-TEXT | Partial: real native fields. No multiline API, shell inFields resolver or IME/Services test. |
| P-A11Y | Not implemented: only model order exists. This pin's AccessKit adapter subclasses the window content view, so a GPUIView-only override is insufficient. Foreign-child merge is a gate. |
| P-120 | No new patch; upstream display-link/virtual-list behavior retained. Actual view caching, display transitions and 120-Hz frame times unverified. |

## Matrix and evidence

PASS is limited to the evidence named; model tests do not certify AppKit behavior.
The standalone GPU verifier returns nonzero for pixel/lifetime/clip/retirement
failures. Timing observations are reported, not silently promoted to display gates.

| S2 row | Status | Evidence / remaining work |
|---|---|---|
| GPUI-CE macOS build | PASS (1.99.1); UNVERIFIED (required 1.96.1) | All priority patch crates + spike linked on 1.99.1. 1.96.1 check stopped on disk. |
| Hit priority: child → above → GPUI → compat holes → below | PASS model; UNVERIFIED native | `hit_order_and_overlay_holes`, `z_order_last_slot_wins` (1.96.1); native view hooks compiled. |
| Drags across slots, capture across republish | PASS model; UNVERIFIED native | `capture_survives_boundaries_and_republish`; no trackpad/NSDraggingSession test. |
| Clip and transform | PASS model + GPU texture clip; UNVERIFIED native | Affine inverse + polygon tests; AppKit shape/hit masks compiled. Native arbitrary affine placement absent. |
| Compat overlay holes | PASS model; UNVERIFIED native | `compat_union_keeps_gap_transparent` passes. WKWebView union mask fixture includes overlay protruding beyond the panel. No real DOM reporting channel. |
| Live resize / docking / compat reflow | UNVERIFIED | Same-transaction hook exists. No ≤1-frame reflow evidence or snapshot-stretch implementation. |
| Display moves / mixed scale / headroom | UNVERIFIED | No second-display run; window follows backing scale at present. S3 must retire display-dependent resources. |
| Accessibility order / VoiceOver | UNVERIFIED, implementation gap | Interleaved order in frame data only; AccessKit foreign children missing. |
| IME in native field | UNVERIFIED | Actual NSTextField slot exists; no Japanese/Chinese composition, Services, dictation or responder trace. |
| Delayed GPU completion | PASS | Producer signal withheld for 120 ms; scene/UI refs dropped; lease remained held, then completed and dropped exactly once. CPU submit did not block on fence. |
| Cached scene re-submission | PASS | Three extra submissions each notify completion; retained scene stays valid. |
| BGRA8 / RGBA16F premultiplied alpha | PASS | Half-alpha red reads `[128,0,0,255]` over opaque black in both formats. RGBA16F is preconverted SDR, not HDR. |
| Retired texture rejection | PASS | Explicit retired source returns error. Real device loss, failed GPU command buffer and two-device rejection still UNVERIFIED. |
| 1,000 distinct texture tiles | PASS offscreen correctness; display performance UNVERIFIED | Real Metal rendering, 150 measured frames after 30 warmups. CPU encode p95 0.429 ms; submit + GPU drain p95 1.243 ms. |
| 1,000 tiles + 60-fps frame producer | PASS offscreen cadence; display gate UNVERIFIED | 120 paced frames; work p95 12.457 ms, 0 work intervals >16.667 ms. Includes producer fixture + extra capture/drain. This exceeds an 8.33-ms 50%-interval headroom target if used provisionally; not paired Electron/corpus playback evidence. |
| 5k virtual rows / long-form 120 Hz | PASS range model; UNVERIFIED GPUI performance | `uniform_list` demo has 5,000 rows. Model range p95 0.000042 ms, which is not paint/frame time. No 120-Hz run or 50%-interval headroom proof. |
| Field menu | PASS tokens; UNVERIFIED/incomplete interaction | One pre-created child window, native search field, spring/exit/fade implementation. Keyboard/typeahead/list/glider, menu focus, current-velocity continuity and tooltip/popover pooling still incomplete. |
| Glass / Reduce Transparency / Increase Contrast | UNVERIFIED native | AppKit material selection and solid fallback code exist. No live light/dark/system-setting captures. Contrast fallback border exists; dense GPUI contrast tokens need full treatment. |
| Headless capture | PASS renderer-only | Three PNGs outside Git, listed below. No native panel capture or full UI oracle claim. |

All seven current contract unit tests pass on Rust 1.96.1 in the standalone
harness, including the disjoint compat-union gap. The full Cargo test rerun was
stopped; it is not being inferred from the smaller harness.

### Artifacts (not tracked)

- `/tmp/pm-shots/s2-texture-bgra.png`
- `/tmp/pm-shots/s2-texture-rgba16f.png`
- `/tmp/pm-shots/s2-1000-tiles.png`

These are renderer captures, not screenshots of the locked desktop. Runtime/build
logs stay under `/tmp/s2-*`, outside Git.

## Kill criterion and handoff

The baseline requires an S2 kill criterion but does not define a numeric combined
S2 verdict. Operational interpretation for this spike: **do not advance S2** if
shared-device texture ownership, native ordering/input/IME/accessibility, resize
reflow (≤1 frame or stretch fallback), or required presentation budgets cannot be
shown to satisfy the contract. A model-only substitute does not satisfy it.

**Verdict: OPEN / NOT PASSED.** The GPU lifetime path has no observed kill failure;
no framework rejection is justified by these results. There is insufficient
native evidence and known unfinished functionality, so P1 approval is blocked.
The offscreen mixed workload also has insufficient provisional 50% headroom.

**S3 risks:** renderer's SDR target is not an EDR viewer; keep engine overlays in
its viewer pass. Validate wgpu-30 objc2 raw-pointer retention, producer format/
encoding, device-wide source retirement, bounded outstanding submissions, display
changes and resize transactions. This prototype draws a test pattern, not engine
frames; it references the earlier engine-host/panel-viewer lifecycle but does not
port those crates or claim their timing for GPUI-CE.

**S5 risks:** real overlay rectangles and GPUI occlusion must come from the same
committed frame, not the synthetic fixed geometry here. Finish keyboard/IME and
foreign-child accessibility. Measure live resize/reflow under load or implement
snapshot-stretch. A compiled WKWebView stand-in does not validate the compat page
channels, lifecycle, deadlocks, full-page masking cost or extension semantics.

Before closing S2: restore sufficient disk; build/test from apps/native on 1.96.1;
finish native affine placement, transient menu behavior, text and A11Y; run the
entire native matrix with a GUI and target displays; measure timeline/presentation
p95 and paired margins; obtain the required orchestrator/Opus review. No subagents
or review delegation were started by this implementation worker.


## Unlocked rerun (2026-11-01)

**Verdict: OPEN / NOT PASSED.** This run adds real-display evidence and fixes two
spike bugs; it does not close the composition, IME, accessibility or headroom gates.
The earlier locked-session results above are retained.

### Conditions and reproduction

macOS 27.2, Apple M5, one built-in 3024×1964 Retina XDR display, unlocked session.
No second display was attached. All builds used Rust **1.96.1** and
`~/Developer/pm-target`. The initial normal-workspace build of S2 and S5 passed.
Another active lane then temporarily made workspace metadata unloadable. Subsequent
builds used copies of the two manifests/lock in `/tmp`, absolute vendor paths and
symlinks to these exact source directories; no shared manifest/lock was edited.
The 4-GiB build guard stopped two attempts; only this verification's duplicate
artifacts were removed before resuming above the limit. The final rebuild passed.
Seven standalone contract tests passed on 1.96.1. This is not a full Cargo-test pass.
Timing is a single development-build diagnostic on a shared machine, not a paired
corpus gate.

Optional demo controls added for reproduction:

```sh
S2_APPEARANCE=light S2_MENU=1 ~/Developer/pm-target/debug/pm-native-s2-spike
S2_PACING=1 S2_MENU=1 ZED_MEASUREMENTS=1 ~/Developer/pm-target/debug/pm-native-s2-spike
```

`S2_PACING` paints 1,000 distinct retained textures plus a fresh 60-Hz producer
source for 650 GPUI frames, then stops requesting frames. It logs the native
viewer drawable's actual Metal `presentedTime`; zero timestamps are not counted
as presentations. `ZED_MEASUREMENTS` is the existing upstream GPUI CPU frame
instrumentation. Viewer-layer timestamps are **not** GPUI-layer GPU completion
measurements. No vendor code was changed.

### On-screen matrix

| Check | Updated result / evidence |
|---|---|
| Native toolbar and dark glass | **PASS appearance smoke.** Real NSToolbar, native field, solid work surfaces, floating bar and child-menu glass visible. |
| Light appearance | **PARTIAL.** Native toolbar, background and field use Aqua with the local override. GPUI/DOM tokens remain dark; the independently created menu still follows the system's dark appearance. No full light-theme pass. |
| Reduce Transparency | **PASS startup fallback.** Effective workspace flag true; bar/menu become solid. Live reaction without restarting remains unverified. |
| Increase Contrast | **PARTIAL.** Effective workspace flag true; solid menu border and native WebKit control outlines visibly strengthen. Dense GPUI contrast tokens remain unfinished. |
| Compat slot and overlay hit | **PASS limited native probe after fix.** Original panel was blank because the web view's initial size was added to a zero-sized autoresizing parent. Initializing the wrapper to the child's frame fixes placement. Clicking the protruding WebKit overlay changed its button to `Clicked`. |
| Cross-slot drag capture | **PASS limited native selection probe.** Dragged selection from the native field across the WebKit slot, then typed; replacement stayed in the original native field (`Native drag aCAPTURE`). No NSDraggingSession/export drag certification. |
| Resize/reflow | **PARTIAL / budget UNVERIFIED.** A real edge drag changed the window from 1180×844 to 1032×772 points; WebKit panel and mask followed at the endpoint. No measured ≤1-frame DOM paint lag and no snapshot-stretch fallback. Docking remains untested. |
| Clip / hit order | **PARTIAL.** Native field stays above viewer; visible WebKit overlay receives a click; child menu stays above GPUI. Arbitrary affine placement, GPUI occlusion over compat overlays under the new tile workload, and every priority/capture combination remain open. |
| Child field menu / half bounce | **PARTIAL.** A real pooled 300×230 child window is visible above the parent. Runtime spring parameters: perceptual duration **0.320 s**, bounce **0.150**, calculated settling duration **0.471456 s**. Search field and four labels appear in AX. This is startup opening, not proof of control-triggered opening, keyboard selection, glider, velocity-continuous interruption or perceptual motion quality. |
| OS IME in P-TEXT | **UNVERIFIED.** Native text entry works. Japanese Romaji and Simplified Pinyin input methods enabled successfully but `TISSelectInputSource` returned **-50** for both. U.S. selection succeeded; temporary methods were disabled. Manual composition/conversion/commit/cancel and Services remain required. |
| Accessibility order | **FAIL.** External AXUIElement walk was trusted and completed. Main content exposes an empty AXGroup, toolbar and window chrome; native field, WebContent and transport are absent. The child menu exposes its search and labels separately. Interleaved slot children are not implemented. VoiceOver was not toggled. |
| Display moves | **UNAVAILABLE.** One display attached; no mixed-scale/display-transition claim. |

Controls on `NSGlassEffectView` now use its supported `contentView` instead of
arbitrary subviews. The pre-26 path checks class availability and keeps ordinary
AppKit contents. Menu interaction still needs a complete independent run.

### Visible texture workload and presentation pacing

650 requested GPUI frames completed in **5.815 s** with the field menu open.
After 30 warmups, 620 upstream CPU frame samples had p50/p95/p99
**6.316 / 6.884 / 7.703 ms**, maximum **13.797 ms**; **4/620** exceeded 8.333 ms.
Within the bounded workload, 638 positive viewer presentation timestamps yielded
606 strictly-positive intervals after warmup: p50/p95/p99
**8.333250 / 8.333292 / 8.333375 ms**, maximum **33.333 ms**, **6** intervals
above 8.5 ms. Zero/non-increasing timestamps were excluded, not promoted to frames.
This demonstrates a 120-Hz presentation cadence for the native viewer during this
workload, with missed intervals. **FAIL provisional 50%-interval headroom:**
CPU p95 6.884 ms exceeds **4.167 ms** at 120 Hz. Full GPUI GPU/scanout timing,
long-form scrolling, view caching and paired engine/corpus playback remain open.

The original offscreen verifier was also rerun while the display was unlocked:
all alpha, fence/lifetime, repeat-scene, clip and retirement checks passed.
1,000 tiles (150 samples): encode p95 **0.462 ms**, submit+drain **2.905 ms**.
60-Hz producer + tiles (120 frames): work p95 **14.601 ms**, **5** frames above
16.667 ms. The offscreen mixed fixture still fails provisional half-interval
headroom and does not replace the visible measurement.

### Captures and restoration

All screenshots remain outside Git:

- `/tmp/pm-shots/s2v-original-dark.png` and `s2v-fixed-dark.png`: blank-panel reproduction and fix.
- `/tmp/pm-shots/s2v-1000-tiles-onscreen.png`: actual displayed workload and glass child menu.
- `/tmp/pm-shots/s2v-light.png`, `s2v-field-menu-light.png`: partial light appearance.
- `/tmp/pm-shots/s2v-reduce-transparency.png`, `s2v-menu-reduce-transparency.png`: solid fallback.
- `/tmp/pm-shots/s2v-increase-contrast.png`, `s2v-menu-increase-contrast.png`: stronger borders.
- `/tmp/pm-shots/s2v-hit-after-events.png`, `s2v-resized.png`: overlay click and resized layout.

System Settings was used after `defaults write` was denied. Both accessibility
switches were observed off before testing and restored off afterwards. Original
`defaults` keys were absent; macOS now stores explicit false values. Attempts to
remove those keys through defaults/CFPreferences were denied or ineffective; the
protected plist also rejected direct removal. **Effective settings are restored;
exact original key absence is not restored.** Global appearance remains Dark;
U.S. input is selected; VoiceOver and Reduce Motion were unchanged. Temporary
input methods were disabled, fixture processes stopped and the GUI lock released.

The gate remains open principally for foreign accessibility, real IME, measured
resize lag/fallback, native ordering coverage, complete menu behavior and headroom.
No framework kill is inferred from a synthetic development-build timing failure.

## S2b P-A11Y (2026-11-01)

**P-A11Y automated gate: PASS. Spoken VoiceOver walkthrough: UNVERIFIED.**
The earlier S2 implementation notes and unlocked rerun above are retained as the
baseline. This section supersedes their foreign-child implementation gap; the
**overall S2 gate remains OPEN / NOT PASSED** for its other outstanding rows.

### Implementation and decisions

| Decision | Rule that picked it |
|---|---|
| Use the documented AppKit fallback. The pinned AccessKit macOS 0.26.3 adapter has no API to attach an NSView as a foreign child. | P-A11Y's native-first selection rule, then its explicit fallback. |
| Wrap the actual content view after AccessKit installs its subclass; preserve its real root and merge at that root. GPUIView forwards AX hit testing, and GPUIWindow/GPUIPanel forward focus to the content adapter. | P-A11Y and N3: correct AXParent, hit and focus relationships across the separate drawing/content views. |
| Publish `Gpui(AXIdentifier)`, native slot views and `RemainingGpui` with the committed composition. Preserve GPUI subtrees and any unnamed children. | §3.1.2: reading order interleaves slots and GPUI; it is independent of z order. Hosts provide direct exposed root anchors. |
| Insert AppKit's unignored elements, including an NSControl's cell; keep WebKit's original remote subtree, frames, focus and actions. | N3 correctness and N8: no duplicate AXUnknown control wrappers or copied DOM accessibility tree. |
| Return native WebKit responder focus / NSControl field-editor focus; reclaim GPUIView as first responder on an AccessKit Focus action. | P-TEXT/P-A11Y: logical GPUI focus alone cannot take keyboard ownership back from a slot. |
| Query AX while locked and leave spoken/visual checks open. | N7 inspectability and the explicit screen-free constraint; no unlock request, screenshots or VoiceOver toggle. |
| Set the spike's application appearance through GPUI as well as its window; make the child menu follow it. Match the overlay mask to its CSS bounds. | N9 / §3.10 native appearance and N3 exact geometry. Solid work surfaces remain solid. |

Vendor implementation: `composition_accessibility.rs`, its pure order planner,
`composition.rs` publication and small window hooks. The per-instance subclasses
add no ivars. Native parent overrides are cleared on removal/unregister; teardown
restores both classes before dropping AccessKit. Layout notifications are queued
after the GPUI draw lock is released. AX getters read retained publication data
without entering the host or taking the window mutex. The proposed upstream
foreign-child API and pin assumptions are recorded in POWERMOVE.md; nothing has
been submitted upstream.

### AX tree evidence without the screen

The external client used `AXUIElementCreateApplication(spike_pid)` and walked
`AXChildren`, roles, titles, values and frames. **AXIsProcessTrusted was true.**
The pre-patch binary reproduced the earlier failure while locked: an empty main
AXGroup, with toolbar/window chrome but no native slots. The patched main root
exposes these direct children, in the published reading order:

| Order | Identifier / role | Screen frame in fixture points (x, y, width, height) |
|---|---|---|
| 1 | `s2-navigation` / AXGroup (GPUI) | 96, 224, 160, 136 |
| 2 | `s2-viewer` / AXImage (slot 1) | 284, 196, 670, 520 |
| 3 | `s2-field` / AXTextField (slot 3) | 293, 205, 252, 28 |
| 4 | `s2-web` / AXGroup (slot 2) | 80, 132, 1180, 792 |
| 5 | `s2-transport` / AXGroup (slot 4) | 560, 678, 220, 44 |
| 6 | `s2-timeline` / AXGroup (remaining GPUI) | 96, 754, 199, 26 |

All six AXParent values match the same retained AccessKit root, and
AXChildrenInNavigationOrder agrees with AXChildren. The native field frame
includes AppKit's border expansion. The web slot's group covers its full-window
view; its layer/hit mask still clips to the disjoint panel/overlay union.
Under it, the actual WKWebView exposes **AXWebArea → AXTextField** named
`WebKit text input`, with frame **984, 263, 223, 28**. The native transport
exposes three AXButtons.

Programmatic focus passes **GPUI → WebKit inner field → GPUI**, then
**GPUI → native field → GPUI**. Each AXFocused setter succeeds, the node reports
AXFocused=true, and the application's AXFocusedUIElement matches that exact
node. The previous GPUI node reports **AXFocused=false** while either native
text field owns the responder, then regains focus on return. A follow-up check
caught and fixed a stale GPUI focus flag: window-key status alone was insufficient.
Successful native responder changes now queue an AccessKit host-focus update
after the AX action releases its tree borrow; key-status updates also check the
actual responder. Native field and web input AXValue writes/readback/restoration pass.
The WebKit button's AXPress changes its title to `Clicked`; the native glass
Play button accepts AXPress. Hit tests use screen points, exercising the actual
AppKit/AccessKit bridge rather than only the composition model.

### Reproduction and validation

```sh
# Builds on pinned Rust, checks contracts, then starts/stops its own fixture.
apps/native/spikes/s2/scripts/check-ax.sh --exercise

# Use an already built fixture binary, or inspect an existing spike process.
apps/native/spikes/s2/scripts/check-ax.sh --no-build --exercise
apps/native/spikes/s2/scripts/check-ax.sh --pid <spike-pid> --exercise
```

The Swift walker is also usable directly for a future foreground check. It never
activates another app, synthesizes input, prompts for permission or requests an
unlock. Its default mode checks structure; `--exercise` also tests native focus,
hit testing, value changes and actions. It skips the system menu bar so unrelated
Recent Items are never included in its output. A missing AX permission returns
BLOCKED with a distinct exit code instead of opening a permission prompt.

The wrapper sources `engine/env.sh`, selects Rust **1.96.1** from `apps/native`,
and uses **only `~/Developer/pm-target`**. It monitors its own builds against the
**4-GiB** stop threshold. Because parallel lanes are changing workspace metadata,
it builds an isolated manifest and copied lockfile in the temporary directory,
pointing at these exact sources/vendor paths. It never edits the shared manifest
or lockfile. No cargo clean, alternate target directory or global setting change.

- Baseline: **7/7** original S2 contracts pass on 1.96.1 before the patch.
- After: **8/8** S2 contracts pass, including a new wide/narrow overlay-leading-edge geometry regression; **2/2** tests of the exact vendor order planner pass.
- Required pinned-toolchain spike build and full isolated Cargo test: **PASS**, 8/8 tests.
- Existing offscreen renderer verifier: **PASS correctness** for alpha, delayed completion/lifetime, repeat submission, clip and retired-source rejection. Loaded locked-session timing remains unsuitable as a presentation gate: 1,000-tile encode/drain p95 3.177/10.418 ms; mixed 60-Hz producer work p95 46.154 ms with 120/120 intervals over 16.667 ms. This is a diagnostic failure of the timing budget, not evidence of a comparative P-A11Y regression or a passed S2 performance gate.
- AppKit-only probe using the exact vendored bridge: **PASS** for NSTextField, multiline NSTextView and NSButton; parent/navigation agreement, reorder/removal, an AccessKit update, unregister and retained-node teardown. Its NSWindow is never ordered on screen.
- External AX structure/focus/value/action checks above: **PASS while locked**, including hit tests for GPUI, both text fields, viewer and glass control, plus the previously clipped overlay label's leading edge. The fixture retains its normal startup activation request behind the lock; AX access for a deliberately inactive/background window was not tested. No unlock was needed.

### Visual fixes and remaining limits

The overlay's CSS left edge is `window_width - 12 - 320`; the old mask began
28 points farther right. One shared geometry helper now drives the native mask
and hit-hole rectangle. The regression checks both the 1180- and 1032-point
fixture widths. This addresses the clipped leading label without moving the DOM.
The spike now sets GPUI's application appearance to Dark (or its existing explicit
light override), applies the same appearance to the native window and child menu,
and reports **NSAppearanceNameDarkAqua for both window and menu** in the dark
fixture. Actual titlebar pixels and light/dark visual quality are **UNVERIFIED**.

Actual spoken VoiceOver next/previous navigation, rotor behavior, focus-ring
presentation, focus notification timing under VoiceOver, real IME/Services,
nonrectangular/affine accessibility geometry and display/resize transitions still
need an attended run. Programmatic focus is evidence for the bridge, not proof
of VoiceOver speech/traversal. This patch inserts at exposed GPUI root anchors;
it does not insert inside arbitrary nested GPUI subtrees. Existing S2 performance,
resize-lag, menu and display gates are unchanged. No P1 approval is implied.

Raw AX/build/test logs are temporary, outside Git; only synthetic fixture results
are summarized here. No screenshots, private baseline text or user data are added.


## S2b unlocked foreground follow-up (2026-11-01)

**S2 remains OPEN / NOT PASSED.** Required Rust 1.96.1 fixture build, contracts
and AppKit bridge probe passed before the on-screen session. The foreground
external AX walker passed with the floating menu closed: published reading order,
parents, native/WebKit subtree, focus transitions GPUI → WebKit → GPUI and
GPUI → native field → GPUI, value restoration, hit testing and actions. With the
floating menu open, one transport-button center hit test hit the menu window;
that occlusion case is retained, not called a transport regression or hidden.

App set A, present and untouched: Finder, Aside, Grok Bot, Notion, Mail,
Claude, T3 Code (Nightly), installed Powermove/helpers, RobloxPlayer,
RobloxMenuBar and figma_agent. Every result below retains this app-presence
caveat; load is the observed one-minute average before → after the case.
Existing Powermove process count changed from seven to five without this lane
closing them. No owned benchmark instance remains. All projects and profiles
were disposable public/synthetic fixtures under `/tmp`.

| Check | Result | Conditions |
|---|---|---|
| Foreground AX walk, no menu | PASS all walker checks; zero failures | A; one-minute load 3.90 → 4.26 over the S2 session |
| Dark titlebar / native menu | PASS visual smoke; both report DarkAqua; black titlebar and dark menu captured | A; same session load |
| Previously clipped overlay label | PASS visual smoke and leading-edge hit test; full “Compat overlay hole” visible; DOM button changed to “Clicked” | A; same session load |
| Live resize endpoint | PASS endpoint smoke: real edge drag 1180×844 → 1032×772; WebKit/mask reflow and hit region followed | A; resize/IME checkpoint load 3.38 |
| ≤1-frame reflow lag or stretch | UNVERIFIED; no frame-resolved DOM paint instrumentation or stretch proof | A; same resize checkpoint |
| VoiceOver speech/next-previous traversal | UNVERIFIED. First-use Quickstart appeared; VoiceOver started without changing its “do not show again” checkbox. Cursor query returned −1728, scripted move −1708; no spoken-navigation pass claimed | A; S2 ending load 4.26 |
| Japanese IME | UNVERIFIED. Romaji selection returned −50; selection remained U.S.; U.S. restore returned 0 | A; resize/IME checkpoint load 3.38 |

Owner's 30-second IME check: choose Japanese Romaji from the input menu, focus
“Native name”, type `nihon`, Space to convert, Return to commit; start another
composition and Escape to cancel. Check candidate placement/caret and cross-slot
focus, then switch back to U.S. Do the spoken VoiceOver walk separately with
VO-Right/Left across GPUI, native field, WebKit and back; programmatic focus is
not a speech substitute.

Captures, outside Git: `/tmp/pm-shots/onscreen-s2-dark.png` and
`/tmp/pm-shots/onscreen-s2-resized.png`. No system appearance, contrast or
transparency setting was changed. Input selection was restored to U.S.; initially
off VoiceOver and its Quickstart process were both verified absent at the end.
No product/vendor code changed in this lane. All foreground processes were owned,
time-bounded and closed. Remaining prior layout, menu, performance and display
contracts are not promoted to passes by these appearance/AX observations.

## 2026-11-01 — P-TEXT/P-120 pass

**Implementation / screen-free verification PASS; display and OS IME gates OPEN.**
“Pass” in this heading does not certify S2, 120-Hz headroom, or P1 approval. The
user explicitly requested finishing with visible checks unverified after the Mac
locked. No after-patch performance or actual Japanese/Pinyin conversion is claimed.

### Changes and evidence

- Native primary/multiline and dense text/number slot APIs now share the composition
  transaction, rectangular clipping, retained accessibility order and AppKit responder
  chain. Multiline uses a scrollable `NSTextView`. Dense editing uses a borderless
  `NSTextField` in the same fixed rectangle; commit-only number validation and
  Escape rollback preserve intermediate/marked input. Shell key and popup navigation
  monitors stop intercepting marked-text events. Spelling/correction, native undo,
  Writing Tools default behavior, Services and emoji use AppKit's implementations.
- Frame-source start is idempotent on the same display; the existing per-display
  CVDisplayLink remains responsible for rate and display transitions. Actual GPUI
  drawable timestamp instrumentation replaces inference from the sibling viewer.
- The texture-headroom fix candidate caches the static native viewer, unchanged
  clip masks and static 1,000-tile GPUI subtree. Its 60-Hz producer remains separate.
  `S2_BASELINE=1` restores the previous fixture's repeated work for paired A/B runs.
  `S2_TIMELINE=1` scrolls a real 5,000-row virtual list every requested frame.
- Rust **1.96.1**, shared `~/Developer/pm-target`: build PASS; **24 Rust tests PASS**
  (2 design, 8 S2 contracts, 12 shell models, 1 appearance/Metal capture, 1 cache
  regression). AppKit text/AX probe PASS: real UTF-16 marked range and commit,
  multiline, spell/undo configuration, numeric parsing, clipped placement,
  dense/native equal rectangle, teardown, and the prior foreign-child AX probes.
  These test APIs are not a Japanese or Chinese input method.
- No display mode, microphone permission, VoiceOver setting or persistent input
  source was changed for these tests. The GUI lock is released; test app is closed.

### Before/after measurements

| Workload / condition | Before | After |
|---|---|---|
| Earlier unlocked S2 1,000 tiles + producer, CPU p95 | **6.884 ms**, FAIL against 4.167 ms | UNVERIFIED |
| This session's unlocked pre-patch reproduction, shared machine, development binary, 619 samples after 30 warmups | p50 **10.527**, p95 **13.040**, p99 **14.143 ms**; **511/619** CPU frames >8.333 ms | UNVERIFIED — lock screen observed before after-patch run |
| Actual GPUI-layer presentations / zero dropped UI frames | No valid pre-patch GPUI-layer timestamp series | UNVERIFIED |
| Scrolling 5,000 virtual rows, 120 Hz / ≤4.167 ms CPU p95 | Ready-to-run A/B baseline | Ready-to-run patched workload; UNVERIFIED |
| Real OS IME / Services / dictation / Writing Tools / emoji | Previous TIS selection returned -50 | UNVERIFIED; capability and manual checks below |

The second baseline is a diagnostic, not a settled/paired corpus result. Do not
compare it as an improvement/regression against the earlier run. Locked-session
single-frame logs are excluded. The headroom failure is **not closed** by a code
change or model test. The fixture's unchanged-view cache is tested, but its visible
frame-time benefit still needs measurement. No 0-drop claim is made.

### Exact reproduction commands and pass criteria

From the repository root:

```sh
# Pinned build, GPU verifier, all 24 Rust tests, native text/AX probe; no visible window.
apps/native/spikes/s2/scripts/check-ptext-120.sh

# Unlocked display: one diagnostic A/B batch (4 runs, ~4.5 minutes, lock released).
apps/native/spikes/s2/scripts/check-ptext-120.sh --display --no-build --runs 1

# Required repeated matrix after the diagnostic passes; one bounded batch at a time.
apps/native/spikes/s2/scripts/check-ptext-120.sh --display --no-build --runs 30

# Unlocked text demo, TIS capability probe, then 45 seconds for the manual check.
apps/native/spikes/s2/scripts/check-ptext-120.sh --ime --no-build
```

The wrapper sources `engine/env.sh`, selects Rust 1.96.1 from `apps/native`, uses
only the shared target and refuses/stops its own build below **6 GiB** free. It
creates temporary metadata under `/tmp` with exact source symlinks, avoiding
another lane's unfinished manifest/lock. It never edits the shared lock. GUI runs
atomically create `/tmp/pm-shots/gui.lock` containing `ptext`, retry held locks every
30 seconds and release it on normal/error exit. Each acquired batch is <5 minutes.
Locked sessions fail immediately; no unlock is attempted. Screenshots are only
`/tmp/pm-shots/ptext-*.png`; logs/results are `/tmp/ptext-*.log` and
`/tmp/ptext-120-results.json`, all outside Git.

Display pass criteria, **for both patched workloads separately**, on a confirmed
120-Hz display: ≥60 seconds of measured presentations after 30 warmup samples,
CPU frame p95 **≤4.166667 ms**, and **0 missed 8.333333-ms presentation intervals**.
A gap ≥1.5 intervals counts as missed refreshes; zero/duplicate/missing drawable
timestamps invalidate the evidence instead of counting as successful presents.
The script requires ≥6,000 CPU samples and at least as many valid presentations.
Keep the app visible and the machine on power/settled without competing timing
apps. Maximum display capability alone does not prove 120-Hz pacing: actual GPUI
presentation timestamps must pass. The baseline workloads are measured and
reported but need not pass. Formal acceptance requires all 30 repetitions, the
specified machines/conditions and paired reference/corpus runs; this synthetic
fixture alone does not certify long-form project playback.

`S2_TILE_RENDER` should occur once per stable-size patched tile run; more render
calls are expected for baseline, resize or genuine child invalidation. Pixel
correctness and producer fence/lease tests must continue to pass. Display moves,
mixed scale and rate changes require a separate two-display check; unavailable
hardware is not a pass.

The TIS helper tries installed Japanese and Simplified Chinese modes, reports
`TISEnableInputSource` / `TISSelectInputSource` statuses, restores the previous input
source and disables only sources it temporarily enabled. It does **not** synthesize
composition or report an IME pass. The demo remains open after that capability
probe; choose Japanese Kana/Romaji or Simplified Pinyin from the input menu for the
following **30-second manual check** (especially if selection reports `-50`):

1. In the multiline Caption field, type `nihon` (Japanese Romaji) or `nihao`
   (Pinyin). Verify marked underline and the OS candidate panel at the caret.
   Space/select a candidate, Return to commit. Pass: expected converted text,
   one insertion, no GPUI shortcut/action firing, working undo.
2. Start a second composition and press Escape. Pass: candidate/composition cancels
   before any field-level Escape action. Tab/click between Caption, Native name
   and Layer name; repeat composition in the dense Layer name field.
3. Click Scale, enter a decimal, Return, then reopen/edit/Escape. Pass: committed
   value survives, Escape restores it, invalid/non-finite input restores it, and
   the field rectangle and neighboring rows never move during either swap.

Additional native-service checks: select text and invoke a system Services action;
show Spelling and Grammar and correct a misspelling; insert one emoji via Edit →
Emoji & Symbols; invoke dictation using the configured macOS shortcut and confirm
insertion; invoke Writing Tools where available and confirm replacement/undo.
Pass only each feature actually exercised. Microphone/account/model availability
or OS settings can make a feature unavailable; record that explicitly. These
checks and keyboard traversal remain UNVERIFIED in this pass.

Remaining S2 work is still open: arbitrary affine/shape native placement, measured
≤1-frame live resize/DOM reflow or snapshot-stretch fallback, all native drag/capture
combinations, display moves, spoken VoiceOver traversal and the complete menu/
transient/motion/system-appearance matrix. P-A11Y's screen-free bridge results are
retained; they do not certify those visible behaviors. No release, push or tag.

## On-screen IME and P-120 batch (2026-11-01, 09:59–10:02)

**OS IME in the native text slot: PASS (Japanese Romaji and Simplified Pinyin,
synthetic keys). P-120 GUI pacing: NOT RUN (the Mac locked at ~10:17, before the
quiet window).** S2 remains OPEN. Binary: existing dev `pm-native-s2-spike`
(built 09:19, sources of `93638bf7b`), `S2_TEXT=1`, built-in 120 Hz panel, AC power,
one-minute load 7–8 (other lanes compiling). One GUI lock per run (`onscreen`).

Why the earlier probes returned −50: `TISSelectInputSource` fails on an input
*mode* whose parent input method is disabled. Enabling the parent
(`com.apple.inputmethod.Kotoeri.RomajiTyping` / `com.apple.inputmethod.SCIM`) and
then selecting the mode (`…RomajiTyping.Japanese` / `…SCIM.ITABC`) returns 0.

| Check | Result |
|---|---|
| Focus | AX `set focused` on the native field: PASS (`focused = true`, frontmost `pm-native-s2-spike`). Latin control `abc` reached the field. |
| Japanese compose → convert → commit | PASS (run 1): `nihon`, Space, Return gave `abc日本語`. Marked text was underlined in the field, and the candidate window was anchored directly under the caret. Run 2 typed literal `nihon` with the mode reported as selected: Kotoeri stayed in direct input after the rapid enable/select. Recorded as a test-harness flake, not an app failure. |
| Japanese cancel | PASS: with live conversion, Escape #1 reverted to kana (`かな`, still marked), and Escape #2 removed the marked text. |
| Pinyin compose → convert → commit | PASS: `zhongwen`, Space, Return gave `abc中文`. |
| Pinyin cancel | PASS: Escape removed the marked `kan a`. Return had selected the whole committed value (standard `NSTextField` end-editing), so the new composition replaced it and the field ends empty. Escape did not roll back committed text. |
| Click-to-focus via System Events `click at` | **Observation, not certified:** the AX click resolved to “text field 1 of group 1”, but the field did not become first responder. The IME then composed against the GPUI view, and its candidate window appeared at the window's bottom-left corner (no caret rect). A real HID click was not tested. If a real click reproduces it, the GPUI view's `firstRectForCharacterRange` with no focused input handler needs a sane rect. |
| Services / dictation / Writing Tools / emoji | UNVERIFIED |
| P-120 display A/B (`check-ptext-120.sh --display --no-build --runs 1`) | NOT RUN: screen locked |

Restoration: the U.S. layout is selected and both temporary parent methods are
disabled. The *effective* enabled/selectable set (U.S., Character Palette,
Press-and-Hold, ironwood) equals the pre-test set. Two inert plist differences remain
in `com.apple.HIToolbox`: an `AppleEnabledInputSources` row for
`SCIM`/`ITABC` (parent disabled, so not in the input menu; `TISDisableInputSource`
returns 0 but does not remove it), and a new `AppleInputSourceHistory` key. The
auto-enabled `com.apple.50onPaletteIM` was disabled again. Captures stay outside Git,
under `/tmp/pm-onscreen/s2/`.

Reproduce: `/tmp/pm-onscreen/s2/ime.py [pinyin]` is a local, untracked driver. It
launches the spike, AX-focuses `text field 1 of group 1`, types through System
Events, captures each step and restores input sources. Rerun P-120 with the
documented `check-ptext-120.sh --display --no-build --runs 1` on a quiet, unlocked
machine.

## P-120 transaction patch (2026-11-01, lane `feat/na-shell2`)

**Implemented; 120 Hz headroom gate still FAIL/UNVERIFIED.** Before this patch
`composition::present` always returned a transaction, so `MacWindow::draw` set
`presentsWithTransaction` on every GPUI frame and never cleared it: every frame ran
`commit → waitUntilScheduled → present` on the main thread.

What changed (vendor `gpui_macos`, see `POWERMOVE.md` P-120 row):

- `composition::register_tracked` hosts return `SlotChange::{Moved, Unchanged}` from
  their present callback; `native_text` reports its own frame/clip/visibility changes.
  The untracked `register` keeps the old behaviour (always transactional).
- `draw` now sets the flag **per frame** and resets it: transactional when slot geometry
  moved, on the first frame after registration, during AppKit synchronous paints
  (`displayLayer:` live resize, tab activation), **or when frames are continuous**
  (previous frame < 33.3 ms ago). Isolated frames (hover, caret, a single edit) present
  from the command buffer without blocking the main thread.
- Hosts: the shell (viewer slot frame/layer/drawable size/visibility) and the S2 spike
  (`contract::slots_moved`) report moves. `S2_SYNC_PRESENT=1` forces the pre-patch
  behaviour; `PM_GPUI_ASYNC_PRESENT=1` forces the first-cut "async unless moved" policy.
- Tests: `spikes/s2/src/p120.rs` (7) via `#[doc(hidden)] composition::probe`, plus
  `contract::p120_moves_only_when_geometry_changes`. (The vendor crate's own tests do not
  resolve offline: a wasm-only git dependency.)

### On-screen A/B (why continuous frames stay transactional)

`check-ptext-120.sh`-equivalent driver: `PTEXT_META=/tmp/<lane> python3
apps/native/spikes/s2/scripts/check-ptext-120.py --display --no-build --p120 --runs 1
--seconds 60` (GUI lock, one batch of 6 × 60 s). Built-in panel at 120 Hz, AC power,
debug build, one-minute load ≈ 6 (other lanes compiling) — diagnostic, not a gate run.
CPU = upstream `frame duration` (main-thread draw + present).

| Workload | Policy | CPU p50 / p95 ms | `waitUntilScheduled` (n, p95) | `nextDrawable` p95 | missed 8.33 ms intervals | zero `presentedTime` |
|---|---|---|---|---|---|---|
| 1,000 tiles | always transactional (pre-patch) | 6.69 / 7.66 | 6,943 × 0.50 ms | 0.035 ms | 149 | 126 |
| 1,000 tiles | async unless moved (first cut) | 6.27 / **10.93** | 0 | **4.14 ms** | 203 | 0 |
| 1,000 tiles | **adaptive (shipped)** | 6.54 / 7.98 | 6,937 × 0.52 ms | 0.036 ms | 188 | 135 |
| 5,000-row timeline | always transactional | 3.25 / 4.51 | 6,918 × 0.12 ms | 0.039 ms | 1,639 | 1,552 |
| 5,000-row timeline | async unless moved | 4.60 / **12.60** | 0 | **8.78 ms** | 226 | 3 |
| 5,000-row timeline | **adaptive (shipped)** | 3.12 / 3.85 | 6,952 × 0.11 ms | 0.038 ms | 1,124 | 1,059 |

A first run (same build, before the adaptive rule) reproduced the async result
(tiles `nextDrawable` p95 4.19 ms, timeline 9.14 ms). **Finding:** an asynchronous
present stays on glass ~3 refreshes (perf-gate-p1 "Latency pass"), so back-to-back async
frames starve CAMetalLayer's three drawables and the main thread blocks in `nextDrawable`
instead — 8–80× the 0.1–0.5 ms `waitUntilScheduled` it saved. Hence the adaptive rule
(rule: measured frame evidence, N8). Notable trade-off: the async path almost never
produced zero-timestamp (never-shown) drawables, while the transactional timeline path
loses ~1,000–1,500 of ~7,000; that is open S2 work (likely the same compositor-latch
behaviour), not addressed here.

Main-thread time per frame for **isolated** frames now excludes `waitUntilScheduled`
(0.1–0.5 ms here); no isolated-frame workload exists in the spike, so that saving is
asserted by the policy tests, not measured. All runs FAIL the provisional criteria
(CPU p95 ≤ 4.167 ms, zero missed intervals, ≥ 6,000 valid presentations).
Logs: `/tmp/ptext-{tiles,timeline}-{sync,async,p120}-0.log`, results `/tmp/pm-shell2/`.

## On-screen rerun 2 (2026-11-01, 10:39–10:52)

**P-120 GUI pacing: FAIL. Japanese IME conversion: PASS, with a screenshot.**
S2 remains OPEN. Conditions: unlocked, AC, built-in 120 Hz panel. The shared
build lock was held, so no lane compiled. One-minute load was 4–7, from
`fileproviderd` and an existing Electron instance. Binary: the existing dev
`pm-native-s2-spike` (built 09:19). `apps/native/spikes/s2`, `crates/shell`,
`crates/design` and `vendor/` are unchanged between its sources (`93638bf7b`)
and `0863a9732`. The run used the documented command from the checkout that
owns `/tmp/ptext-120-build`, with an identical script:
`check-ptext-120.sh --display --no-build --runs 1`. It held the GUI lock as
`ptext` for one 4 × 65 s batch.

| Workload (120 Hz, 65 s) | CPU frame p50 / p95 (ms) | Positive presents | Zero (unknown) timestamps | Missed 120 Hz intervals | Verdict |
|---|---|---|---|---|---|
| tiles-before (baseline) | 6.163 / 7.001 | 7,258 | 112 | 119 | FAIL |
| tiles-after (P-120 cache; 2 tile renders) | 5.924 / 7.131 | 7,188 | 182 | 203 | FAIL |
| timeline-before | 3.207 / 3.797 | 6,291 | 1,078 | 1,117 | FAIL |
| timeline-after | 3.126 / 3.604 | 6,579 | 790 | 826 | FAIL |

All four rows are `presentation_evidence_valid: false`, because zero
timestamps are present, and all fail the ≤ 4.167 ms CPU p95 and zero-miss
criteria. The timeline workload is under the CPU budget, but it misses ~11–15%
of 120 Hz intervals. The P-120 view cache cut tile renders from 7,401 to 2 but
did not lower CPU p95 or misses. Raw rows are in
`/tmp/pm-onscreen2/s2/ptext-120-results.json` (not tracked).

**Japanese IME** (`S2_TEXT=1`, synthetic keys through System Events,
Kotoeri Romaji):
`nihon` → Space → Space shows the candidate window anchored under the native
field with **日本語** selected. Return commits it: the field value read via AX
is `日本語`. Driver change versus `/tmp/pm-onscreen/s2/ime.py`: Kotoeri only
switched out of direct input after the focused field ended editing once. The
earlier "run 2 flake" reproduced twice as literal `nihon` with the mode
reported as selected. Sending Return + Right Arrow after `TISSelectInputSource`
fixed it. A real user who switches input source while a GPUI-hosted
`NSTextField` is already first responder may hit the same thing; this is
unverified with a real keyboard. Afterwards U.S. was reselected and the
temporarily enabled Kotoeri parent was disabled again.
Captures (outside Git): `/tmp/pm-onscreen2/s2/crop-converted.png`
(candidate window), `japanese-3-committed*.png`, and the P-120 batch
screenshots `ptext-{tiles,timeline}-{before,after}-0.png`.
