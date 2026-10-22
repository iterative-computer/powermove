# Proxy generation and representation policy (Phase 4a)

## Decision PROXY-1 (orchestrator, beta)

- **Still pixels are today's pixels.** Paused frames, settled seeks and
  exports show exactly today's pixels: the original through today's source
  conversion, bilinear preview-canvas minification and browser upscale, at
  Full, Half and Quarter. No proxy or reduced-decode pixels appear there
  unless the user explicitly picks a draft (the viewer's Proxies choice, or a
  draft export). Analysis always reads originals.
- **Playback may degrade, measured.** While playing, a source that this Mac
  measurably cannot decode in real time may show a ready proxy or a reduced
  decode. This is a beta decision. The engine returns to the exact original
  within one frame of pausing (the first non-stand-in paused frame).
- **Always Play Original** gives today's behaviour throughout playback: it
  forbids proxies and partial-resolution decode for that asset.
- **Reduced decode on stills** is allowed only where it is pixel-exact
  against today's path. None is enabled: every paused representation is the
  full original.

Today's viewer substitutes a disposable editing copy (at most 1280 pixels,
VP9) when preview quality is below Full or Auto quality is adapting. Under
PROXY-1 the engine draws the original there instead; the paired test
`engine-proxy-viewer-parity.spec.ts` records the editing-copy difference as
evidence and gates on today's original path. Sources Chromium cannot decode
(ProRes, most HEVC) play through today's playback conversion; like section
25's video class, those are not compared pixel-for-pixel with today.

## Generation and intent

`pm-proxy` owns whole-clip proxy recipes, validated generated media, the disk
guard and the per-machine representation decision. The project stores the
user's intent (Create proxies, size, format, Media menu, Always Play Original),
not the machine profile or a generated file path. The job engine accepts
`JobSpec::GenerateProxy` in `PriorityClass::Generation` on its dedicated
utility lane. It resolves `MediaRead` and `CacheKind::GeneratedMedia` through
host capabilities; a request carries only an asset id. The lane pauses its
FFmpeg child during playback or an export and resumes it afterwards. The
child and its temporary files are RAII owned and removed on cancellation,
failure and engine shutdown.

The versioned recipe includes requested format and size, actual even output
dimensions, source bit depth and alpha. Alpha and source depth above 10 use
ProRes 4444, preserving up to 12 encoded bits from deeper sources; other
preferences are ProRes 422 Proxy, 422 LT or HEVC Main10.
The bundled LGPL FFmpeg's VideoToolbox encoders generate MOV with PCM audio.
Output validation checks codec profile, dimensions, depth, alpha, rotation,
field order, clean aperture, HDR/ICC metadata, display aspect, source color
tags, timecode and reel, exact video frame count and rational PTS, and audio
stream count, PCM coding/depth, rate, channels, start and duration. For PCM
sources it also checks decoded sample count and canonical sample payload hash.
RGB sources are converted explicitly to tagged limited-range YCbCr. Odd
source dimensions round the encoded raster up while the output sample aspect
preserves its display aspect. An output that cannot meet these conditions
fails its job and is never stored.
The source's BLAKE3 hash is checked before and after encoding, so a linked
file changing during generation does not get bound to stale content identity.
An original image sequence enters the same job through a machine-local JSON
manifest with `powermoveSequence: 1`, a rational frame rate, and ordered
absolute frame paths with their BLAKE3 hashes. Its aggregate key hashes the
rate and ordered frame hashes, never the paths. The producer verifies every
original frame before and after encoding, stages temporary links to those
frames, and checks output depth, alpha, frame count, and exact rational PTS
before committing the MOV. It never reads the legacy VP9 sequence rendition.
Homogeneous sRGB, Display P3, and known linear EXR frames retain their source
primaries and transfer tags. Mixed color spaces, other ICC profiles and
scene-linear highlights are converted from the original frames into
BT.2020/PQ 16-bit staging images, with scene-linear 1.0 mapped to 100 nits;
the output can represent positive highlights through PQ's 10,000-nit ceiling.
Arbitrary EXR primaries use their chromaticities and white point, and other
ICC profiles use ColorSync. The original frames remain the source for
ordinary exports. A proxy is a bounded display representation, so negative
scene values and values beyond the PQ ceiling cannot be retained in it.

The generated-media store keys by full source BLAKE3 plus the complete version-2 recipe.
MOV and metadata use atomic renames; lookup rehashes bytes and refuses an
incomplete or changed object. `find_for_source` discovers a validated proxy
after moves, relinks or relaunch without a source pathname. The host supplies
the cache location. Generation checks available space before starting and
throughout encoding, pausing below max(20 GiB, 10% of the volume). Playback,
export and low-disk pauses emit a nonterminal `JobState::Paused` status with
a reason and resume automatically when the condition clears.

`MachineProfile` records GPU p95, disk throughput and power state;
`DecodeMeasurement` records each asset's measured decode p95. The automatic
decision uses decode cost, GPU headroom, power state and viewer device-pixel
size; decode measurements include source I/O cost. It changes representation
only at playback start or clip boundaries, or when the machine profile changes
(for example, a change to battery power). Paused Automatic uses the complete
original at Full, Half, and Quarter. If the measured original fits the frame
budget, the policy keeps its exact pixels at smaller viewer sizes too.
`Originals` remains offline when its source is offline;
it never substitutes a proxy. `source_for` requires the original for analysis
and normal export, and allows proxy pixels only for an explicit draft export.

## Desktop and viewer integration

The job engine uses the packaged FFmpeg pair exposed through
`PM_FFMPEG_DIR/bin` or `POWERMOVE_FFMPEG` beside `ffprobe`. Main's
`GeneratedMedia` capability supplies the directory frozen when a job is queued.
`EngineProxyService` serializes shared-store mutations, scopes originals and
draft proxies to their owning project, checks complete proxy checksums using
the media-store worker pool, and persists locations, measured decode decisions
and explicit deletions outside the project. A malformed machine-state file is
reported by name rather than overwritten. Failed jobs require an explicit retry.

The renderer controller installs the appendix D project settings, generated
media location, viewer Media menu, asset commands and Always Play Original.
Only intent and Always Play Original are undoable project state. Actual frame
quality supplies the viewer label. Generation status stays unchanged during
playback. Offline export uses the existing native message and requires an
explicit draft choice; ordinary exports and analysis retain original bindings.

The viewer resolver uses ready content-addressed proxies for explicit Proxies,
offline originals and measured slow playback. It freezes choices at playback
start and clip boundaries, upgrades paused Automatic to original pixels, and
honours project-scoped persisted deletion even when another project retains
the same file. Integrity lookup and full proxy rehash run on a bounded background
worker: the first viewer frame does not wait for disk validation. A completed
lookup is polled into the view state; a changed file identity invalidates a
frozen playback choice and requires validation again. It never participates in
the export resolver. `Always Play Original` is carried with the asset into the
render plan so it also forbids partial-resolution decoding, not just proxy files.
The resolver merges live source and proxy decode p95 measurements with machine
GPU and power readings. Missing reduced-size and proxy decode costs are sampled
on a separate bounded worker from their actual file and decode purpose. That
worker pauses at playback and resumes while idle; its results stay machine-local
and trigger a fresh viewer choice after playback ends.

The legacy video path uploads the selected video's full raster as a WebGL
texture, samples it bilinearly at level zero into the preview canvas, then lets
Chromium scale that canvas to its CSS box. A 1920-pixel video can be cropped
inside a 640-pixel composition, so canvas size is not an upper bound on video
source density. Reduced playback decode is chosen per asset and checked again
against each layer's projected source density. An insufficient quarter tier may
rise to a measured half tier, then to the full original. Projective placement,
paused frames, and assets pinned to the original retain full source texels.
An existing project's exact editing-copy appearance remains a separate parity
requirement when today's Chromium viewer selects an editing rendition.

Original-source cold measurements run on the probe utility lane, with playback
and export pauses, no decoded-frame cache, and separate sequential p95 and seek
latency. Short clips explicitly report insufficient samples. Measurements and
machine identity remain local. The viewer uses the measured reduced/proxy cost
ladder only when sufficient samples exist; an unknown cost does not prove that
an alternative fits the frame budget.

The crate tests cover store identity,
offline/export selection, disk threshold, VideoToolbox ProRes Proxy/LT/HEVC10
and alpha 4444 transcoding, VFR exact PTS, and odd-dimension RGB display
aspect on macOS. The job service test exercises host handle mint/resolve,
playback pause/resume and persistent generation.

The 16-bit RGBA PNG chart test produced a 12-bit ProRes 4444 proxy with exact
24000/1001 frame timestamps and a measured maximum ΔE2000 of 0.0563 across
40 sampled color patches (three original frames, half-size output).

## Runtime machine profile

The interactive `Composer` owns a machine-local profile cache. It derives the
default `Generated Media` folder from the parent of
`POWERMOVE_PROXY_LOCATIONS` (or uses an absolute `PM_GENERATED_MEDIA_DIR`),
then starts a utility-QoS worker. The worker creates the folder and measures
write throughput with an 8 MiB fsynced, automatically removed temporary file.
It reads the power source initially and checks it again every 30 seconds.
Neither disk I/O nor `pmset` runs in the viewer's frame path.
The worker rereads the machine's selected generated-media location every
30 seconds and measures the selected volume when that location changes.
Location and power changes advance the profile generation and reselect the
playback representation without putting file I/O in the render path.

When Metal exposes `TIMESTAMP_QUERY`, the viewer samples one in 16 uncached
renderer frames, with at most three readbacks in flight. A one-workgroup
compute pass at each end of the renderer's command buffer writes GPU
timestamps; a later submission resolves them, and the existing device-poll
callback consumes the result asynchronously. This excludes CPU encoding and
queue-wait time. The measured span covers the renderer's GPU commands; final
viewer placement and cached-preview copies are outside it, and unrelated GPU
work interleaved within the span can increase it. Unsupported devices or
invalid timestamps leave the GPU measurement unavailable; CPU wall time is
never substituted.

`Composer::machine_profile()` returns `None` until the disk/power sample and
eight valid GPU samples are available. Thereafter GPU p95 uses a rolling
64-sample window. `machine_profile_generation()` changes when the profile
becomes ready, power changes, or GPU p95 crosses a 0.25 ms bucket. A caller
can use that signal to reevaluate representation at playback start or a clip
boundary. `gpu_headroom_ms` uses the policy's 16.667 ms reference period;
the representation choice uses the supplied content frame rate for its actual
playback budget. The profile is not written to a project or persisted across
engine launches. The resolver combines this profile with live original decode
costs and background measurements of actual reduced and proxy decode paths.
Unknown costs remain unknown; they do not qualify a tier as real-time.

## Sustained app measurements

`apps/desktop/e2e/engine-proxy-performance.spec.ts` records paired 60-second
Automatic playback of 4K long-GOP HEVC 10-bit and H.264 4:2:2 10-bit, a
ProRes/HEVC proxy-to-paused-original first-frame transition, and one hour of
native playback. The paired rows require real-time presented cadence, zero
native late skips, and native cadence no slower than the measured current
viewer. Measurements begin after the real automatic assessment and any
required generation settle while paused. The hour row samples OS RSS for
Electron and both engine processes, together with native lease, media-cache,
and preview-cache usage each minute. It checks process stability, a
post-warmup plateau and a regression trend bound; the latter is a leak
heuristic because memory samples can be autocorrelated.

Prepare six-second or longer `hevc10-longgop-4k.mov`,
`h26442210-longgop-4k.mov`, and `prores-4k.mov` at 3840×2160 and 24 fps in
`PM_PHASE4A_PERF_MEDIA` outside the Electron lease, then build the desktop
app. Run the dedicated `playwright.phase4a-performance.config.ts` under
`pm-electron-lock` from `apps/desktop`; it disables the ordinary Playwright
global build and extends the suite timeout beyond the one-hour run. Missing
prepared sources fail the measurement rather than marking a row skipped.
