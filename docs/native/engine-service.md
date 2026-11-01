# pm-engine-service

P1 package 6 implementation, macOS, pinned Rust 1.96.1. This is an in-process
Rust service and a windowless export-child entrypoint. **The P1 performance and
viewer gates remain open.** Headless correctness results below do not establish
physical playback continuity, scrub latency, or speaker/display synchronization.

## API and ownership

`apps/native/crates/engine-service` is a native-workspace member through the
existing `crates/*` glob. It depends on `pm-node` with default features disabled:
its public Rust audio/clock/font modules are reused; no napi entrypoint is built.
No engine, kernel, shell, spike or vendor source is changed by this package.

- Create `Gpu` and `EngineService::start` on a worker. Inject
  `service.metal_device()` into GPUI (P-DEVICE). `start_with_fonts` accepts an
  existing `pm_node::fonts::render::RenderFonts`; the main-thread host must keep
  servicing that broker. `fonts_changed()` invalidates paused viewers.
- On main, `layers::configure_layer(MainThreadMarker, layer, device)` configures
  the caller's retained CAMetalLayer: RGBA16Float, EDR, extended-linear Display
  P3, two drawables, acquisition timeout, no transaction presentation. Device and
  pixel format are checked when attaching. The shell owns below-slot placement,
  clipping, drawable size and CATransaction coordination; the service never
  creates a window.
- `viewer(ViewerConfig)` creates an independent main/source/popout viewer.
  Composition identity stays fixed; retire/recreate to switch it. Up to eight
  viewers share one renderer/device and a fair worker. Each has its own
  transport/audio controller. Popouts that should mirror transport must receive
  the same commands; independent source-monitor playback is also possible.
- `Viewer::publish(&Published)` consumes the kernel's COW snapshot. Older
  revisions and different document sessions are refused. Whole immutable tree
  adoption is deliberate: missed change events need no patch recovery path.
  Forward kernel publication events to this method; the service does not own a
  second document authority or an event subscription thread.
- `Viewer::media` installs relinked original/proxy mappings and rotates content
  identity even without a document edit. Normal viewer rendering uses originals.
- `configure(Output)` retires size/display/headroom generations.
  `layers::screen_output` queries display id, current headroom and potential
  headroom on main. Call it from screen/EDR-change notifications and publish
  changed output in the shell's placement transaction.
- `command(TransportCommand)` queues ordered, validated controls (64 maximum;
  contiguous seeks coalesce). `scrub(Ticks)` pauses then wakes immediate exact
  rendering, independently of display ticks. `shuttle(key, from_pause)` follows
  shipped J/K/L decline and saturation: ±1x → ±2x, direction change → ±1x,
  repeated ±2x consumed, stopped keys decline unless explicitly allowed.
- `display_tick(target_host_seconds, interval_seconds)` accepts CADisplayLink
  phase; the worker keeps that phase between callbacks and counts expired
  targets. Without phase the provisional worker schedule is 60 Hz. This is not
  a certified refresh-rate/adaptive-quality scheduler.
- `status()` returns latest clock, levels, frame identity, failures and counters.
  `capture_next()` requests diagnostic half-float readback; normal presentation
  performs no CPU pixel readback. Physical presentedTime and GPU completion are
  separate, optional observations. Missing presentation stays missing.
- Explicitly `retire()` viewers. Drop `EngineService` on a worker: it stops and
  joins rendering/audio and drains GPU work. Drop/join is not a UI event action.

With the default `kernel` feature, `Snapshot` is the real pm-kernel snapshot.
`--no-default-features` offers a small immutable snapshot struct for standalone
CLI embedding, without a kernel dependency. `Headless::{new,with_fonts,frame}`
uses exactly the same final Metal shader offscreen; `export_reference` independently
calls `pm_export::render_frame`. Font-aware headless rendering belongs on a worker
while its main-thread broker is pumped.

## Rendering and overlays

Working frames use `pm_render::Renderer::render_into` into retained
`pm_gpu::bridge_out::ExportedSurface` textures. The final pass decodes signed,
premultiplied engine E encoding, converts linear BT.709 to P3, premultiplies and
writes RGBA16F. It preserves negative and extended values; transient display
headroom is not used as a destructive highlight clamp.

One LRU holds up to eight exact working frames, with a nominal 128 MiB cache
capacity (at least one frame for larger outputs). Keys include content identity,
revision, exact engine ticks and dimensions. Separate viewers get separate
content identities; asset changes rotate them. The source pool is capped at 11
textures. There are at most three submitted consumers globally and one per viewer.
A completed command retains its source lease until its callback is released.
Resize/display/edit changes serialize against the final submission check; already
submitted Metal work drains while its results are rejected if stale. Retirement
cannot recall a drawable already submitted to WindowServer.

`OverlayGeometry` contains up to 64 normalized line segments with linear P3
colours. `safe_areas` is time-invariant. `overlays(stamp,time,geometry)` accepts
paused geometry only at the matching revision/time. `locked_overlays(provider)`
installs a **native Rust** geometry evaluator: it runs on the render worker with
the same document, composition and ticks as the picture, immediately before the
same final pass. It must not call GPUI/JS or wait for main. A viewer-tools layer
still needs to bind actual selected-layer transforms to this interface.

`PlaybackLayers` creates retained CATextLayer/CALayers for timecode, playhead and
stereo meters. The shell attaches/styles them above the slot and pumps `update`
from its native display callback using project-rate timecode, clock and meter
values. It does not invalidate GPUI. Physical layer/picture alignment remains
unverified in this package.

## Audio and exports

The service directly reuses `pm_node::present::audio::AudioLink` (which owns
`pm_audio::Playback`) and `present::clock::TransportClock`: lazy device warm-up,
30 ms project-update throttle, asynchronous controls, output-latency timestamps,
loop entry, epochs and host-clock fallback. An audible start holds picture time
until a new audio epoch arrives. Silent compositions use the host clock. Pause
and seek cannot adopt the previous run's device epoch. The inherited audio-start
timeout is used; fallback is visible in status.

`jobs::ExportRequest::new` freezes the kernel document/revision, `JobSpec::Export`,
media catalog and destination. A worker calls `ExportHandle::start(executable,
request)`: that executable starts itself with `--engine-job`. Integrators must
handle that argument before GPUI startup by calling `jobs::run_child()`.
The crate's own binary demonstrates this dispatch. Private payloads use inherited
pipes, not command-line JSON or files. Status/progress uses the existing
`pm_protocol::priority::JobStatus`. Explicit cancel signals the child, and
`wait()` reaps it. Dropping a live handle kills/reaps it; use explicit cancel/wait
for graceful removal of temporary output.

The child renders on its own worker while main services the reused CoreText font
broker. It uses `pm-export` planning, encoding, verification and atomic no-clobber
publication. Only `settings.draft_from_proxies=true` can select proxies; otherwise
only catalogued originals resolve. Missing originals refuse export rather than
silently substitute proxy pixels. The caller must correctly classify media
provenance. Compat producers are explicitly refused until the P2 runtime host
exists. Caller project-font registration/replay and extension definitions are not
yet serialized to the child. Export GPU playback preemption/slicing across the
child boundary is also still required; this first API uses `Unrestricted`.

## Decisions and picking rules

| Decision | Rule |
|---|---|
| Reuse AudioLink/TransportClock in a plain Rust dependency; keep bridge consumers intact | §3.3 lifecycle reuse, N5: delete only after the last production consumer and replacement gate |
| Adopt COW kernel snapshots, reject revision/session regressions | §3.2 single writer, N3 correct authored state |
| Reuse S3b same-queue texture leases and exact cache; add media/session identities | N1 instant scrubbing, N3 no approximate/stale cache hits, N8 no Node or pixel bridge |
| Keep the EDR layer separate from GPUI SDR; retain engine device objects | §3.3 and P-DEVICE; S3 viewer/export colour evidence |
| Preserve extended values, track headroom as output identity | N3 viewer=export; S3's highlight-clamp failure must not return |
| Evaluate locked geometry on the worker, draw it in the picture pass | §3.3 zero skew and N8; no main-thread per-frame gizmo dependency |
| Use protocol jobs in a same-executable child, explicit original-only resolver | engine-boundary §2.6/§5 and §1 export-safety contract |
| Windowless native tests; report simulated clocks separately | N7 and §5 evidence integrity: GPU completion is not scanout |

## Validation

Run from the repository root with `source engine/env.sh`,
`CARGO_TARGET_DIR=$HOME/Developer/pm-target`, and `cargo +1.96.1`:

```sh
cargo +1.96.1 test --manifest-path apps/native/Cargo.toml -p pm-engine-service -- --nocapture
cargo +1.96.1 test --manifest-path apps/native/Cargo.toml -p pm-engine-service --no-default-features -- --nocapture
cargo +1.96.1 clippy --manifest-path apps/native/Cargo.toml -p pm-engine-service --no-deps -- -D warnings
cargo +1.96.1 fmt --manifest-path apps/native/crates/engine-service/Cargo.toml --package pm-engine-service --check
```

The FFmpeg build's library directory is supplied as an rpath to the service/test
binaries. Tests use the unchanged public shape-primitives corpus fixture, copied
from the S3 preparation path, never a private project.

Final verification: **16/16 tests pass** with the default kernel adapter, and
**16/16 pass** with `--no-default-features`. Clippy with `-D warnings` and package
formatting pass. The existing `block 0.1.6` future-compatibility warning remains.

Measured headless correctness: **57,600 pixels**, ΔE2000 **p99 0.030891**, **max
0.054050**, compared to an independent export working-frame render with the f64
P3 reference. The simulated device-callback harness has **120 matched-time
samples**, mean absolute offset **0.000 ms**, p95 jitter **0.000 ms**, including a
160 ms simulated output latency. This is clock arithmetic/epoch evidence, not a
physical §5 A/V pass. Empty samples stay unmeasured; constant bias is not hidden.

The suite covers forced publication/resize during encoding (actual stale
submission rejected), real delayed Metal completion retaining its source,
cache hits and revision/size/session invalidation, J/K/L/seek/step/loop,
worker-evaluated moving overlays, rapid J/K/L and 100-input scrub bursts, proxy safety, 30-frame child export completion
and cancel after progress (no partial output or temporary directory retained).
All validation is headless. No on-screen batch, screenshot or GUI lock is used.

## Remaining gates and integration

Wire shell/kernel publication events, below-slot transactions, native display
callbacks, layer styling, source-monitor media selection and viewer-tool transform
bindings. Finish resource snapshots (definitions, project fonts, extension
activation and generation pinning) for windowed and job/headless hosts. P2's
windowless compat activation/virtual-time host is a separate dependency.

Complete PQ/HLG encoded output, BT.2390/mastering policy, 8/16/32-bit preview
selection, automatic resolution policy, cache memory accounting and export GPU
slicing. This package currently presents Float16 extended-linear P3, including
engine-decoded PQ/HLG content; it does not claim a PQ/HLG mastering output mode.

Run real audible playback and matched device/scanout A/V capture, AppKit scrub
latency, 60-second zero-drop/headroom soaks, popouts and physical mixed-display
moves. Take the shared GUI lock for every on-screen batch. S3/S3b measurements
are prior evidence, not measurements of this service. The required machines,
corpora, paired Electron comparisons, repetitions, orchestrator verification and
Opus review remain outstanding. No P1/colour/performance gate is declared passed.
