# RFC 02: `FrameRequest` and `FrameDescriptor`

Status: draft, version 0. Module: `pm_protocol::frame`. Gates Phase 2;
freezes when Phase 2 starts. Owners: Engine and platform (bridge side),
Viewer and settings UI (viewer side).

## Purpose

How anything asks the engine for a frame, and the one small message that
accompanies each presented frame (plan sections 8, 14, 18). A request names
what to render and under which quality inputs; a descriptor names which
pooled surface holds the result and everything the viewer needs to present
it correctly: presentation time, the exact revision, color space, what is
on screen, and overlay geometry at that revision so handles never lead the
pixels.

## Types

| Type | Meaning |
|---|---|
| `FrameRequest` | Composition, time, `FramePurpose`, `PriorityClass` (contract 12), output size, optional region of interest, scale (0.01 to 16), `PreviewResolution`, `MediaPreference`, `OutputColor` (contract 4), `Precision`, `MotionBlurKind`, optional `ViewOverride`, `draft_3d`, `draft_render`, and whether to also produce the depth and ID surface and overlay geometry. The envelope's revision is the minimum revision to render (read-your-writes) |
| `FramePurpose` | `viewer`, `scrub` (bypasses the jitter buffer), `thumbnail`, `agentCapture`, `export` (also the synchronous compatibility path for `render.renderFrameTo()`) |
| `ViewOverride`, `EditorCamera`, `Projection`, `Shading3d` | Preview-only: editor camera, shading mode, X-ray, Local View |
| `MotionBlurKind` | Which `motionBlurSamples` count applies: export, alpha export, preview playing, preview paused, or off |
| `FrameDescriptor` | `{poolIndex, poolGeneration, seq, pts, revision, colorSpace, format, quality, overlay?, depthId?}`, sent after the command buffer completes |
| `FrameQuality`, `Representation` | What is on screen: the resolution actually rendered, the lowest representation any visible source used (original, reduced-resolution decode, proxy, optimized media, kept render), native 3D standing in for Blender, served from the memory cache. Drives the viewer label ("Half · Proxy") |
| `OverlayGeometry`, `OverlayLayer`, `PathCommand`, `Caret` | The composition rectangle in view pixels, per-layer world matrices, bounds quads and selected paths, and the text caret |
| `DepthIdSurface` | Pool index and seq of the frame's `rgbaf16` depth and object-id surface (section 8) |
| `FrameRelease` | `{poolIndex, seq}`: the viewer called `VideoFrame.close()` and the imported texture's per-use release callback ran |
| `ViewerState` | Size in device pixels, DPR, zoom, pan, resolution, media preference, canvas color space, view override, Play Every Frame |
| `PoolLedger` | Engine-side bookkeeping: a surface is never written while held, and a release must name the exact `seq` |

## Invariants

Checked by `FrameRequest::validate` and `ViewerState::validate`:

1. Output size is 1 to 16384 px per edge; a region of interest is non-empty
   and inside the output (region of interest applies to the final output
   only, section 8). Scale is finite and in 0.01..16.
2. **Exports and agent captures are final.** They carry no view override,
   render at full resolution, never use preview motion-blur counts, and
   produce no overlay geometry or depth surface.
3. **No proxy pixels by accident** (section 14.6, Proxies bar). An export
   reads proxies only with `draft_render` set ("Render using: Proxies
   (draft)"), and a draft render must read proxies. Agent captures never
   read proxies and never take `draft_render`.
4. **Exports never use draft 3D**: a missing Blender surface fails the
   export. Agent captures may (`blender: "pending"`, section 20), and so may
   thumbnails and the viewer.
5. Exports run in a job class (not interactive or agent snapshot) with
   export or alpha-export motion-blur counts or none. Agent captures run in
   the agent snapshot class. Viewer and scrub frames are interactive and
   use the viewer output color.
6. X-ray applies only in draft shading (wireframe, solid); it multiplies
   opacity by `XRAY_OPACITY` (0.45) and writes no depth.
7. Viewer state: positive finite DPR and zoom, finite pan, size 1 to 16384.

Descriptor rules, enforced by the engine and the bridge:

8. `seq` increases by one per frame produced on a session; `revision` is the
   exact revision rendered and is at least the request's minimum.
9. `poolIndex` refers to the pool generation in `poolGeneration`
   (contract 3, `SurfacePool`); a descriptor from an older generation is
   stale and dropped by the viewer.
10. A surface is not written while held. `PoolLedger::present` refuses a
    held index and `PoolLedger::release` rejects an unknown index, an
    unheld index, or a mismatched `seq` (tested).
11. `allReferencesReleased` is used only for teardown; per-frame release
    uses `FrameRelease`.

## Versioning

Version 0 until Phase 2 starts. The descriptor is on the hot path; adding a
field after the freeze costs bytes on every frame and is a version bump.

Measured (M4 Max, release build, Rust codec with validation and the
finite-float scan, 200,000 iterations, median of three runs while other
builds loaded the machine (load average 8 to 12);
`cargo test --release -p pm-protocol --test wire -- --ignored`):

| Message | Bytes | Encode | Decode |
|---|---|---|---|
| Frame event without overlay or depth surface | 243 | 369 ns | 1,228 ns |
| Frame event with overlay (one layer, one path, caret) and depth surface | 748 | 957 ns | 3,785 ns |

These are Rust-side numbers. Main decodes descriptors in JavaScript; S1
measures that cost under main-process load.

## What Phase 0 must confirm

- **S1 (frame bridge):** the pool format (`bgra8` with blue-noise dither or
  `rgbaf16`) by banding and memory; the pool size (maximum ring + jitter
  buffer + 2); the exact per-use release API behind `FrameRelease`; that
  `sendSharedTexture` within its 1000 ms timeout and the descriptor's
  JavaScript decode fit the S1 gate (p99 jitter ≤ 4 ms, zero drops over
  60 s at 4K 60 fps, pointer-to-pixel ≤ 33 ms); and that overlay geometry at
  the frame's revision keeps handles from leading pixels during layer drag,
  3D orbit, and walk. If S1 falls back to C′ or B, `PlatformSurface`
  (contract 3) changes; this contract does not.
- **S2:** whether the viewer's `rgbaf16` path preserves values above 1.0 on
  an HDR display (S1 gate, third bullet), which decides whether
  `ViewerColorSpace` needs an extended-range variant before the freeze.
- **S8:** that interactive priority on `FrameRequest` is sufficient for the
  contention gate (contract 13 carries the rest).

## Open questions

1. Whether overlay geometry should be a separate event keyed by revision
   rather than an optional descriptor field, so frames without selection
   changes stay at the 243-byte size. S1 measures both.
2. Whether `scale` is redundant with `output_size` for the viewer (it is
   needed for exports with a custom size). Kept for now; the viewer sends
   the effective scale.
3. Whether `Representation` should be per source in the descriptor (for a
   per-clip offline or proxy badge) rather than the lowest across sources.
   Appendix D only needs the lowest; per-clip status comes from the media
   index.

## Implementation notes (production bridge, Phase 2)

Draft changes made while building the production frame bridge (plan
section 18 with amendments 4 and 5; design notes in
[`../bridge.md`](../bridge.md)).

**Needs the contract owner's sign-off.** These are not additive: the four
new fields are required (`FrameDescriptor.view_seq` and `duration`,
`ViewerState.edr` and `view_seq`; `duration` is an `Option` but the field
itself must be present), the descriptor gained validation, the ts-rs types
and wire fixtures were regenerated, and `PROTOCOL_BUILD` changed from
`protocol-15e365d7043a8bfe` to `protocol-16c8e6e125019103`. Every lane that
constructs these structs or speaks the protocol must regenerate and rebuild
after the merge; draft contracts interoperate only between equal builds.

- **`FrameDescriptor.view_seq`** and **`ViewerState.view_seq`**: the viewer
  numbers its states; each descriptor echoes the state it shows. With
  `revision` it tags a frame completely, so overlays drawn from its geometry
  match its pixels during view gestures (zoom, pan, orbit, walk), which
  change no document revision.
- **`FrameDescriptor.duration`**: `Some` for timed (playback) frames, shown
  from `pts` for that long and presented by the viewer's jitter buffer at
  their predicted display time; `None` for frames shown at the next refresh
  (paused, scrub, gesture). This replaces inferring presentation from the
  transport state, which races at play and pause.
- **`ViewerState.edr`**: the canvas's display can show values above 1.0
  (CSS `dynamic-range: high`). The engine presents through an `rgbaf16` pool
  when it is set and the lease allows, and `bgra` with the blue-noise dither
  otherwise (amendment 4). `ViewerColorSpace` keeps its two values: an
  `rgbaf16` pool's format already says the values are extended.
- **`FrameDescriptor` validation**: a positive duration, a viewer format, and
  finite overlay geometry.
- **Release (`FrameRelease`)**: S1 confirmed the API. The viewer calls
  `VideoFrame.close()` after submitting the draw that read the frame and
  releases the slot when `queue.onSubmittedWorkDone()` resolves; the per-use
  release callback runs before the GPU finishes reading and is not used.
  Frames the viewer never drew are released as soon as they are closed.
- **Transport (amendment 4)**: `Event::Frame` and `Event::SurfacePool` go from
  the engine to the viewer over the viewer's `MessageChannelMain` port;
  main is never on the per-frame path. Main sees each surface once over Mach
  and imports it once.

Open question 1 (overlay geometry as a separate event) stays open: the test
source sends one overlay layer per frame, and frames without selection
changes still carry it. The descriptor's size with one layer is in
[`../bridge.md`](../bridge.md).

Later production findings (details and measurements in
[`../bridge.md`](../bridge.md)):

- **A page that goes away never reports its imports released.** Electron
  drops the renderer's release notice once a page unloads, so main reclaims
  the surfaces of a reloaded, navigated, crashed, or closed receiver after a
  grace period once its own import is released. `FrameRelease` and
  `allReferencesReleased` are unchanged for live pages.
- **Sizes settle before they reach the engine.** The viewer sends a new
  `ViewerState.size` only once it held still for 150 ms; zoom, pan, and the
  view override still go at once. A live window resize otherwise replaces
  the pool at every step.
- **`ViewerState.edr` follows the hosting window**, and a setting can turn
  it off (8-bit presentation on an EDR display).
