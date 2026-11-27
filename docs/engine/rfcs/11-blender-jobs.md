# RFC 11: The Blender job protocol, snapshot, and surface format

Status: draft, version 0. Module: `pm_protocol::blender` (plus
`EngineFact::BlenderResult` in `patch`). Gates Phase 2; freezes when
Phase 3a starts (Blender surface compositing is a Phase 3a item; S7 informs
it). Owner: 3D (`pm-blender`), with Engine and platform for host services
(Blender supervision, contract 14).

## Purpose

Compositions with `render3d.enabled` get each 3D layer's pixels from a
user-installed Blender (plan sections 8, 9, 17). Today the renderer prepares
one Blender render per 3D layer per motion-blur sample through one worker,
in 8-bit PNG, and any edit during an export invalidates prepared frames
(appendix A items 26, 31, 34). In the engine:

- the **job engine** builds Blender snapshots from `pm-eval` and
  `pm-geom3d`, verified equal to today's `renderSnapshot` output
  (section 25, the Blender snapshots class);
- **host services** run them in a supervised Blender worker with a scrubbed
  environment, and return one **surface** per layer, frame, shutter sample,
  and nested composition;
- level 2 surfaces are **16-bit RGBA PNG through AgX with straight alpha**
  (decision 15); level 1 retains today's PNG8/dither 1 under owner amendment
  22. Both are cached by content key, never re-rendered within a job, and
  composited by the engine as a source node with its crop rectangle;
- model operations (recipe generation, `.blend` import) run through the same
  host services and come back as an engine fact the renderer commits as one
  undoable edit, dropped if the revision moved.

## Types

| Type | Meaning |
|---|---|
| `BlenderJobRequest` | `Request::BlenderRender` (job engine → host, answered `ack`): job id, `PriorityClass` (contract 12), the snapshot, the worker-script hash, referenced assets, settings, the surfaces wanted, whether a frame's layers may be batched in one worker invocation, and the output cache handle |
| `SnapshotRef`, `SnapshotEncoding` | The snapshot in a cache file behind a capability handle, addressed by its BLAKE3 hash; encoding `renderSnapshotJsonV1` (today's `renderSnapshot` JSON) |
| `BlenderAsset` | A texture or `.blend` source the snapshot references: asset id, content hash, read handle |
| `BlenderSettings`, `BlenderEngine`, `RequestedDevice` | From the composition's `render3d`: EEVEE or Cycles, samples, preview flag and preview scale, denoise, device as stored (`auto`, `cpu`, `gpu`) |
| `SurfaceRequest`, `ShutterSample` | One surface: layer, composition path (outermost first, depth 1 to 8), evaluation time with the shutter offset included, shutter sample index and count, size, cache key |
| `BlenderSurface` | `Event::BlenderSurface` (host → job engine), one per finished surface: job, request index, key, full size, crop rectangle, the PNG's handle and byte size, the device Blender actually used |
| `ResolvedDevice` | CPU, Metal, OptiX, CUDA, HIP, oneAPI with the device name. `auto` is resolved and recorded per job (appendix A item 33) |
| `BlenderProgress` | `Event::BlenderProgress`: completed and total surfaces |
| `BlenderStatus` | `Request::BlenderStatus` → `blenderStatus`: available, `major.minor.patch`, error. `supported()` checks Blender 4.5 or newer |
| `BlenderModelJob`, `BlenderModelOperation` | `Request::BlenderModel` (renderer or document host → host, answered `ack`): `generate {recipe}` (the document's `ModelRecipe` JSON, validated by the kernel schema) or `import {source, object?}` from a `.blend` asset, at a document revision |
| `BlenderOutputFile` | One file a model operation produced (GLB, textures) in the host's scratch cache |
| `EngineFact::BlenderResult` (contract 5) | The model operation's result: job, the revision it was requested at, output files, and the worker's manifest JSON |
| `SurfaceKey` | Every input of a surface's cache key (section 14.7); `key_bytes()` is a stable positional encoding the job engine hashes with BLAKE3 |
| `PINNED_OUTPUT`, `WORKER_FLAGS`, `MIN_BLENDER_VERSION`, `limits` | The worker's fixed output settings and flags, and the section 3 limits |

## Invariants

1. **Pinned output** (section 17, not configurable and not on the wire;
   the worker script and the cache key assume it): PNG, 16-bit, RGBA,
   `view_transform = 'AgX'`, `look = 'None'`, sRGB display device,
   `dither_intensity = 0`, gamma 1, scene-following image color
   management, fast compression. The engine ignores the PNG `gAMA`, `sRGB`,
   and `iCCP` chunks. AgX's piecewise-sRGB output codes are E (contract 4),
   so the engine reads them as E-encoded, straight-alpha values and
   premultiplies on input (contract 3, `import_blender_surface`).
2. **The worker runs least-privileged** (section 19, security):
   `--background --factory-startup --disable-autoexec --offline-mode`
   (the Powermove worker still runs through `--python`), a scrubbed
   environment instead of main's full environment (appendix A item 29),
   scratch `BLENDER_USER_*` directories, and external texture paths packed
   or reported at import.
3. **Limits** (section 3; may be raised, never lowered), checked by
   `BlenderJobRequest::validate` and `BlenderModelJob::validate`: Blender
   4.5 or newer; 1 to 4,096 surfaces per job (the nested-frame limit); at
   most 512 assets; snapshots at most 64 MB; surfaces 1 to 8192 px per
   edge (Blender jobs reject larger sizes today; native 3D silently caps,
   appendix A item 27); nesting depth 1 to 8; shutter index below its count;
   samples 1 to 4,096 (final) or 1 to 512 (preview); preview scale 0.1 to 1;
   recipes 1 byte to 8 MiB; `.blend` object names 1 to 160 characters, and
   the source `.blend` among the job's assets. Inputs up to 256 MB, a queue
   of 16 jobs, and the 10-minute timeout are enforced by host services.
4. **Output cap.** Today's 512 MB output cap cannot hold one 8192² 16-bit
   RGBA surface (512 MiB of pixels), so surfaces stream per layer: the cap
   applies per surface, `MAX_OUTPUT_BYTES_PER_SURFACE` = one 8192² RGBA16
   frame plus 16 MiB of PNG overhead (`BlenderSurface::validate`).
5. **Crop.** A surface's crop rectangle lies inside its full size; pixels
   outside it are transparent.
6. **Content-keyed cache.** A surface's key covers the snapshot hash, the
   worker-script hash, the sorted asset hashes, the Blender version, the
   settings, the resolved device, the preview flag, the evaluated camera,
   the size, the layer, and the shutter sample (section 14.7). Asset order
   does not change the key; any listed input does (tested). Within a job a
   surface is rendered once; edits never invalidate an export's prepared
   surfaces because the job renders from its own snapshot session.
7. **A missing surface fails the export** (`ErrorCode::BlenderFailed`, job
   scope); exports never deliver draft shading in place of a Blender
   surface. Agent frames may stand in native shading and say so
   (contract 12, `BlenderUse::Pending`).
8. **Batching only when proven.** `batch_layers` may be set only after S7
   shows batched and per-layer surfaces are equal; several workers run only
   within the memory lease (section 17). Blender's own memory is outside
   leases; workers are bounded by measured RSS (contract 8).
9. **Blender during playback** pauses or throttles (contract 13,
   `BlenderThrottle`). Cancel stops Blender.
10. **Model results respect the revision** (section 6, engine-originated
    facts): the renderer commits a `BlenderResult` as one undoable edit and
    drops it if the document is no longer at the requested revision, as
    today. The renderer mints asset ids on commit; the engine never writes
    the document.

## Versioning

Version 0 until Phase 3a starts. The snapshot encoding versions separately
through `SnapshotEncoding`: a change to the worker script's input is a new
variant and a contract bump, and the worker-script hash in every key keeps
old cached surfaces from being reused. Raising a section 3 limit is
compatible for readers but still a bump after the freeze, because an older
host would reject what a newer engine sends.

## What Phase 0 must confirm

- **S7 (3D):** 16-bit Blender surfaces with straight alpha after AgX into
  the job engine; batching equality (decides whether `batch_layers` is ever
  set); orthographic framing (appendix A item 38: Blender fits the wider
  dimension, native uses a fixed half-height of 3; which to preserve is
  Jude's decision) and glTF source texture orientation (appendix A
  item 39); the PNG compression level for "fast compression"
  (`PinnedOutput::compression_percent` is `None` until S7 measures write
  cost).
- **Blender snapshot parity (section 25):** that `pm-blender` snapshots
  equal today's `renderSnapshot` output for every 3D corpus frame (preview
  and final, every shutter sample, nested compositions). The harness is
  built against `SnapshotEncoding::RenderSnapshotJsonV1`.
- **Phase 0 security evaluation:** a network-denying sandbox profile for the
  Blender worker (`sandbox-exec` is deprecated, a feasibility risk). The
  flags and the scrubbed environment in invariant 2 do not depend on it.
- **S8:** that pausing Blender during playback (the default
  `BlenderThrottle::Pause`) is required, or that throttling to one worker
  keeps zero drops while Blender EEVEE renders.

## Open questions

1. Whether surfaces should be written as raw RGBA16 (or EXR half) instead of
   16-bit PNG to avoid PNG encode and decode cost at 8K. Decision 15 fixes
   16-bit PNG through AgX for cutover; S7's write-cost measurement may
   reopen it after cutover.
2. Whether the job engine or host services own the surface cache directory.
   Current answer: host services mint the `BlenderSurfaces` cache handle and
   the job engine owns eviction under its lease.
3. Whether agent `BlenderPolicy::Render` requests (contract 12) share the
   user's Blender queue of 16 or get their own lane. Section 17 gives agent
   exports their own lane; Blender jobs inherit the requesting job's class.
4. The mapping of today's `render3d` sample and denoise fields onto Cycles
   and EEVEE settings stays in the worker script; whether the snapshot or
   `BlenderSettings` carries engine-specific extras (EEVEE shadows,
   Cycles light paths) is decided when Phase 3b ports the worker.

## Decisions made while connecting it (Blender lane, 2026-10-17)

Version 0 is unchanged on the wire (no type changed); these settle how the
types are used. Design notes: `docs/engine/blender.md`.

1. **One request per Blender frame.** A `BlenderJobRequest` carries the
   snapshot of one composition at one time and size; its `surfaces` are
   that frame's layers (all with the same composition path, time, shutter
   sample, and size). Shutter samples and nested compositions are separate
   requests.
2. **Where the bytes are.** `SnapshotRef.handle` and `output` are the
   requesting session's `cache {blenderSurfaces}` handle. The snapshot is
   `snapshots/<hex BLAKE3>.json` in that area (canonical JSON, sorted keys,
   `JSON.stringify` numbers); a converted image is
   `assets/<hex SHA-256>.<ext>` with the same cache handle; every other
   `BlenderAsset` has a `mediaRead` handle for its asset. A finished
   surface is `<hex key>.png` in the area.
3. **Hashes.** `worker_script_hash` is SHA-256 of
   `engine/crates/pm-blender/worker/worker.py`; host services refuse any
   other. Asset hashes are SHA-256 of the bytes. Surface keys are BLAKE3 of
   `SurfaceKey::key_bytes`.
4. **The answer marks the end.** `blenderRender` is answered `ack` after
   the job's last `blenderSurface` event, or with the error that ended it
   (`blenderMissing`, `blenderFailed`, `cancelled`). Events precede the
   answer on the same port.
5. **Cancel.** The job engine sends `cancelJobs {kind: job, job}` with the
   Blender job's id to host services, which stop Blender (kill the worker's
   process group) and answer `jobsCancelled`. An engine launch ending
   cancels its Blender jobs.
6. **Crop.** Host services send the full surface as `crop`; the engine finds
   the occupied rectangle when it decodes (today's `cropSurface` rule), so
   main never inflates PNGs.
7. **Pinned output additions.** Compression 15 (S7's fast compression),
   metadata stamps off (byte-stable files, S7 defect 7), and the resolved
   device in the manifest. `PINNED_OUTPUT.compression_percent` stays `None`
   in `pm_protocol` (not on the wire); pm-blender's worker script is the
   authority.
8. **Batching.** `batch_layers` is set: S7 measured equal surfaces, and the
   lane's test reproduces it with the pinned worker.
9. **Devices.** The surface key uses the device Blender resolved; before a
   job has run on a machine, Cycles on the CPU is known and everything else
   is learned from the first job's manifest and recorded with the cache.

## Existing-project output (bridge continuation, 2026-10-18)

Owner amendments 21–24 override invariant 1 for compat level 1. An optional
`compatibilityLayers: string[]` root field in the canonical snapshot lists
rendered object ids with compat below 2 (missing compat means 1). Those layers
use PNG8/dither 1, matching today's worker. Other output settings stay pinned;
level 2 retains PNG16/dither 0. This is an additive JSON-v1 field; the build and
worker-script hash handshake require the matching worker, and both snapshot
and script hashes change surface keys. The snapshot corpus comparison checks
all original fields and independently derives this additional list.

Decoded surfaces retain straight as well as premultiplied codes, with both
allocations charged to the cache. Level 1 uploads normalized RGBA8 straight
codes, including RGB under zero alpha, and blends as premultiplied, matching
today's ImageBitmap/WebGL behavior and Canvas2D alpha-derived crop with 2 px
padding. Level 2 keeps the exact premultiplied floating-point path. Both paths
are tested independently; existing app captures and still exports match
today's own captures exactly (100% within 2 CV, max 0 CV), rather than being
compared only with an ideal composite of the raw Blender surfaces.

## Explicit preview during playback (THREED-6, round 7)

`BlenderJobRequest.foregroundCapture` defaults to false and is valid only
with Interactive priority. Only an explicit `scene3d.renderFrame` cache
request sets it. Host services resume the serialized Blender worker while
such a request is queued or running, then restore the playback pause on
completion or cancellation. Work already ahead of it in the same FIFO may
finish first. Automatic previews and ordinary captures keep the default
pause policy. This prevents a caller awaiting `renderFrame` during playback
from waiting for a pause it never requested; transport and viewport shading
remain unchanged.
