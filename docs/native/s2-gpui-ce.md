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
