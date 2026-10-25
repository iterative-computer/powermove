# Source preview monitor

With Use New Engine enabled, the viewer requests source-time frames through
`EngineSourceMedia` and the native snapshot transport. These requests identify
an asset independently of the active composition. Source capabilities resolve
original media through the same binding resolver as composition and export,
including ProRes originals. Existing missing-object recovery follows that
resolver’s policy; legacy browser renditions never replace native source pixels. Rust decodes the source,
applies Interpret Footage, and selects its exact frame; a seek at the source
end holds the final frame. Image-sequence sources share composition’s persisted
frame slots and colour reading. Rotation, clean aperture and pixel aspect
are applied when fitting original video into the monitor or filmstrip. The legacy monitor and asset fields used by older
extensions remain available when the setting is disabled.

The monitor presents native video through `SourceVideoPresenter`: decoded
planes with the shared colour reading, or an engine-converted sRGB frame when
that reading requires native conversion. Its private video element consumes a
native MediaStream, not a media URL. Stills use engine-rendered pixels in an
image surface. SVG sources currently use the existing SVG raster bridge.
Coded planes are packed once, from the decoder's buffer straight into the
snapshot's shared-memory segment, in a WebCodecs buffer layout (NV12, I420A,
I422 or I420P10). Every source frame is a buffer-backed `VideoFrame`: an
Electron shared-texture import wedged the renderer's main thread on the next
import, so the monitor no longer uses IOSurfaces. Main assigns each delivery
timestamp before the raw VideoFrame is created. Startup replays retain that stamp, and presentation acknowledgements
match it exactly: retiming a VideoFrame clone changes its JS wrapper but not
the timestamp Chromium consumes from its underlying frame.
When a coded source exceeds four times the monitor's device pixels, or its
colour/geometry requires conversion, the engine resamples it before delivery.
It uses composition's declared sampler: Lanczos-3 for paused frames and
trilinear filtering for playback. A minified frame rebuilds one kept mip
pyramid on the GPU (released at rest); unminified frames reuse the source
texture directly. Pausing requests a Final-quality repaint. A rearmed
resolution media query repaints a paused monitor when DPR alone changes,
independently of CSS resizing. Source snapshots wait for the host's answer
for the file they read, so a pending binding is never reported offline.

Source audio has a separate native voice and clock. Original stream facts
define the monitor’s duration and audio presence; audio sample zero retains
its presentation offset, and the clock continues through silent portions.
The engine reports that endpoint to the UI without rewriting authored clip
trims. Opening the monitor does not replace composition audio.

Asset changes invalidate the open monitor's reading, cancel stale painting
and status work, and repaint at its current time and play state. Metadata
identity excludes generated posters to avoid refresh loops. A generation
rejects late replies after seeks, asset switches, interpretation edits and
closure. Cancelling a submitted composition snapshot also releases its GPU
readback ticket; removing queue metadata alone would exhaust the readback ring.

Source posters and filmstrips use bounded caches and cancellable native jobs.
Batch filmstrips share one GPU submission/readback while retaining exact source
frame selection. Interactive seeks keep an exact decoder worker warm; source
replacement releases that worker and its cached interpretation.

Validation lives in `engine-source-refresh`, `engine-source-guard`,
`engine-source-media`, `engine-source-compositor` and `engine-source-performance`
under desktop `e2e`, plus Rust `source_snapshot`, `sampling` and presenter cancellation
tests. Colour corrections are measured against an independent reference in
`quality-ledger.md`; legacy byte equality is not the correctness definition.
The source performance gate measures alternating native/legacy app sessions
with cold and warm workloads, including 4K HEVC and ProRes. Renderer-only
microbenchmarks do not establish app latency. A delivery stamp proves which
request reached the screen, not which picture, so the gate accepts timings
only with the campaign's untimed correctness evidence: one native session
captures every measured target after its acknowledged delivery and compares it
with FFmpeg's independent decode of the intended and neighbouring frames,
bound to the build, fixture hashes, targets and geometry
(`e2e/helpers/source-frame-correctness.ts`).

Source in/out, J/K/L and reverse playback remain transport features to add;
the current monitor exposes play/pause and scrubbing.
