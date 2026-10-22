# Proxy generation and representation policy (Phase 4a)

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
only at playback start or clip boundaries. Paused Automatic prefers the
original in place. `Originals` remains offline when its source is offline;
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
the same file. File identity changes trigger validation and invalidate cached
viewer pixels. It never participates in the export resolver.

Original-source cold measurements run on the probe utility lane, with playback
and export pauses, no decoded-frame cache, and separate sequential p95 and seek
latency. Short clips explicitly report insufficient samples. Measurements and
machine identity remain local. The standalone automatic policy includes GPU,
power and device-pixel inputs, but its full measured reduced/proxy cost ladder
is not yet connected to the viewer's selection; that remains a Phase 4a gap.
The tiled still cache exists in `pm-media`; drawing only visible tiles and a
native original-image-sequence producer also remain integration gaps.

The crate tests cover store identity,
offline/export selection, disk threshold, VideoToolbox ProRes Proxy/LT/HEVC10
and alpha 4444 transcoding, VFR exact PTS, and odd-dimension RGB display
aspect on macOS. The job service test exercises host handle mint/resolve,
playback pause/resume and persistent generation.

## Runtime machine profile

The interactive `Composer` owns a machine-local profile cache. It derives the
default `Generated Media` folder from the parent of
`POWERMOVE_PROXY_LOCATIONS` (or uses an absolute `PM_GENERATED_MEDIA_DIR`),
then starts a utility-QoS worker. The worker creates the folder and measures
write throughput with an 8 MiB fsynced, automatically removed temporary file.
It reads the power source initially and checks it again every 30 seconds.
Neither disk I/O nor `pmset` runs in the viewer's frame path.
The current probe measures the default cache volume even if a project's
generated media is redirected to another volume.

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
engine launches. Full automatic choice still needs measured reduced-size and
proxy decode costs in addition to the existing original decode samples.
