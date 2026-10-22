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

### Version note: `ViewerState.vsync` (amendment 18, after `protocol-16c8e6e125019103`)

An additive, optional field (`PROTOCOL_BUILD` `protocol-4263a0f5f5f748a0`):

```rust
pub vsync: Option<VsyncHint>, // #[serde(default)], TypeScript `vsync?: VsyncHint | null`
pub struct VsyncHint { pub host_time: HostMicros, pub period_us: f64 }
```

- **What it is.** The refresh grid of the display showing the viewer, on
  the shared host clock: refreshes (BeginFrame times, the viewer's rAF
  timestamps) fall at `host_time + n × period_us`. The viewer fits it from
  its animation frames (`VsyncEstimator`) and maps it to the host clock
  with the offset it already keeps for the jitter buffer.
- **Why.** The engine starts a timed playback frame at a fixed phase of
  that grid (half a refresh after the BeginFrame by default), so its GPU
  work lands after Chromium's compositor drew that refresh instead of
  contending with it at viz's deadline (S1: viz's draw took 2.6 to 2.7 ms
  instead of 0.6 ms for frames that reached the display a refresh late).
- **Compatibility.** Optional on the wire: a peer built before the field
  sends no `vsync` key and decodes as `None`; the engine then renders at
  the due time as before. Old TypeScript that builds a `ViewerState`
  without it still type-checks. That covers decoding only: the build id
  still moved (`protocol-16c8e6e125019103` → `protocol-4263a0f5f5f748a0`),
  and draft contracts interoperate only between equal build ids, so the
  version handshake refuses a peer built on the old id. Every lane
  regenerates and rebuilds against the new id when this lands, and its
  merge is coordinated with the other lanes'.
- **Not a new view.** A hint-only change keeps `viewSeq` and does not make
  the engine redraw a paused frame. The viewer sends it when first known,
  when the period moves by more than 0.1% or the phase by more than 250 µs,
  and at most once a second unless the display link's phase jumped.
- **Invariant.** `period_us` is finite and within 2,000 to 100,000 µs
  (`ViewerState::validate`).

### Version note: `FrameQuality.stand_in` (long-GOP scrub lane, after `protocol-a6aa78d4b8a57c0d`)

```rust
pub stand_in: bool, // #[serde(default)], TypeScript `standIn: boolean`
```

- **What it is.** Set on a scrub frame (a paused frame after
  `seek {scrub}`) in which at least one video layer shows the nearest
  decoded frame of its source while its exact frame decodes, as
  `SeekMode::Scrub` ("nearest fast frame first") allows and as today's
  viewer draws a video element's last decoded picture. The engine draws the
  frame again with the exact video as soon as it decodes; that frame has
  `stand_in` false. Never set on timed (playback) frames, export frames,
  or agent captures (docs/engine/scrub.md).
- **Why.** "The viewer always says what is on screen" (section 14.1), and
  measurements must tell a stand-in from the exact frame at the same `pts`.
- **Compatibility.** A peer that does not send the key decodes as `false`.
  The build id moved (`protocol-a6aa78d4b8a57c0d` →
  `protocol-2e87c1a0e6d5735f`), so draft-contract peers rebuild against it,
  as with the `vsync` note above.

### Version note: optional headless state (bridge round 4, 2026-10-18)

`ViewerState.headless?: boolean | null` defaults to presentation when absent,
false or null. True releases color/depth presentation pools and suppresses
presentation draws while preserving the session, transport and pm-audio.
Headless clocks are revision-tagged and sent only when the composition/media
are ready for commands. Normal viewer state reattaches to the same clock;
there is no implicit seek, Play, epoch change or audio-device restart. Explicit
presentation frame requests fail without a viewer; offscreen captures use the
snapshot path. See RFC 16 for the renderer lifecycle. The generated build is
`protocol-2f8d7cab61015d02`; addons and TypeScript must be rebuilt together.

### Version note: native 3D viewport chrome (3D round 5)

`ViewerState.chromeSelection` carries selected and active layer IDs. A change
advances `viewSeq`, just like an editor-camera change. A requested `depthId`
pool now contains two RGBA16F surfaces for each color slot. Its slot ownership
is coupled to the color slot, including out-of-order releases; it is not
reused by frame-sequence modulo. The transport pool bound is 64 planes,
while color slots remain bounded at 32; maximum configured render-ahead
therefore also has a complete depth/coverage pair for every color slot.
Color completion alone does not release that slot. The viewer retains the
currently displayed chrome pair through replacement and through the overlay
WebGL read fences. Retired pool imports wait for those readers as well;
late imports of a retired generation are released immediately.

`DepthIdSurface.encoding = "chromePlanesV1"` identifies these stage-sized,
GL-row-order planes. `poolIndex` names depth; `coveragePoolIndex` names IDs and
selection in the same pool generation. The depth target is Depth32Float.
Its IEEE f32 window-depth bits are split 11/11/10 into RGB, each divided by
2048, so RGBA16F transport preserves every bit. Coverage RG stores the
one-based 22-bit topmost object ID in the same binary fractions. Coverage B
is 0, .5 or 1 for none, selected or active, from a separate double-sided
pass without occlusion by other objects. Alpha is 1 on both planes.

The descriptor's `legend` maps IDs to engine layer/object IDs; `glyphs`
contains evaluated light/camera world geometry, and `selection` contains
expanded model-group selections. All are evaluated at the color frame's
revision, time, editor camera and Local View. `depthId.seq` must equal the
color frame's sequence. The viewer requires the matching revision, view
sequence and depth generation before consuming them. Grid occlusion and
outlines use these planes; glyph box selection uses their glyph metadata.
The new-engine viewport does not reconstruct a scene in three.js.
This contract builds as `protocol-e0820481c9e45c9e`.

`previewControl` and `previewStatus` are session-scoped events for the
presenter's Blender preview. A positive generation identifies an explicit
render/cancel request; `automatic` distinguishes the independent generation
sequence of automatic Material/Rendered previews. Controls include revision,
composition, time and shading. A ready status includes the exact frame sequence; the API completes after that
frame reaches the viewer. Addons and generated TypeScript rebuild together
because these draft contracts require equal protocol build IDs.

### Scene diagnostics and cache-only preview (THREED-6/7, round 7)

`FrameDescriptor.scene3dMeta` contains `{composition, layers, warnings, error}`
groups from the render plan that supplied this frame. The viewer applies them
on presentation, alongside the descriptor's sequence and revision. Empty
warnings clear the previous warning array; construction errors set the existing
scene-error/missing metadata. Shared scenes use one group for all their object
layers. No scene redraw or renderer geometry evaluation supplies these records.

`PreviewControl.cacheOnly` requests native Blender cache preparation without
changing transport or viewer shading. It carries the full composition `size`
and optional editor `camera`. Solid/Wireframe, playback, and viewerless
`renderFrame` use this path. Its explicit `PreviewStatus` has `cacheOnly: true`
and may be Ready with no `frameSeq`, after the native cache contains all
requested surfaces. A regular paused Material/Rendered request still completes
only on the matching presented frame. Render requests never pause or seek.

### Version note: optional viewer canvas (decision V1, 2026-10-19)

`ViewerState.canvas?: ViewerCanvas | null` (Rust `canvas:
Option<ViewerCanvas>`, `#[serde(default)]`) is the WebGL canvas today's
viewer draws the view into:

```rust
pub struct ViewerCanvas {
    pub width: u32,  // backing size: previewRenderSize, the explicit
    pub height: u32, // preview resolution, or the crop's renderWidth × renderHeight
    pub crop: Rect,  // the composition part it holds (the whole composition, or the crop)
    pub css: Rect,   // its CSS box in stage CSS pixels
}
```

With it the engine renders the viewer frame at that canvas's size and
today's `outputScale` (`width / crop.width`), and scales it into the box
snapped to whole device pixels with bilinear sampling, as Chromium shows
today's canvas (docs/engine/bridge.md, "Today's viewer canvas"). Absent or
null: the composition at zoom × DPR × the preview resolution, as before.
`width` and `height` 1..16384, `crop` and `css` finite with positive size
(`ViewerState::validate`). A change of it is a new view (`viewSeq`). It
replaces the `svgScale` field this branch first added, so the branch moves
the build id once (`protocol-8433dea27e348b5a`); addons and TypeScript are
rebuilt together.

### Version note: chrome selection and viewer canvas together (3D merge, 2026-10-22)

`ViewerState` carries both optional fields above: `chromeSelection` (3D
chrome) and `canvas` (decision V1). Each keeps its own meaning; neither
depends on the other. The combined contract builds as
`protocol-50a5c6ee43a404de`.
