# Native corpus P1

> WIP follow-up: the historical GLSL diagnosis is being corrected after finding a missing compiler helper in the frozen runner bundle. See [the handoff](handoff/corpus-p1.md) for current status, validated migration fixes and unfinished work.

**Gate P1 remains OPEN. All discovered entries are accounted for; verification is incomplete.**

Status: 2026-11-01; native base `ff42d751fbf6`.

The corpus contains **92 entries: 47 library slots and 45 external PMV entries**. These are entries, not deduplicated authored projects. **34 confirmed extension-dependent entries are deferred to P2.** The remaining 58 candidates include 19 inputs that could not be copied safely; their extension status is unverified.

**39 candidates opened through pm-kernel, completed an MCP edit and saved/reopened successfully. 26 passed all nine sampled stages.** One of those 26 retains an ancillary capture warning; 25 have no recorded capture limitation. No kernel-open, edit or save defect was observed among the 39 admitted candidates.

## Stage totals

All 58 candidates are included. P2 is excluded. Capture/open failures below include 18 macOS dataless placeholders and one unavailable source; these fail before kernel open and are not parser-defect evidence. Blocked stages did not complete because an input or earlier stage was unavailable.

| Stage | Pass | Fail | Blocked |
|---|---:|---:|---:|
| Capture / open | 39 | 19 | 0 |
| Render | 28 | 7 | 23 |
| Play | 28 | 0 | 30 |
| Scrub | 28 | 0 | 30 |
| Audio mix | 34 | 1 | 23 |
| MCP edit | 39 | 0 | 19 |
| Save / reopen | 39 | 0 | 19 |
| MP4 + decode | 26 | 9 | 23 |
| ProRes + decode | 26 | 9 | 23 |

## Failure classes

The four main classes below account for the 32 candidates that did not pass all sampled stages. Counts are distinct entries, not individual failed stages.

| Class | Entries | Finding / next action |
|---|---:|---|
| Input unavailable for capture | 19 | 18 dataless placeholders and one unavailable source. Obtain locally materialized copies, then retry. Eligibility remains unknown until their contents can be read. |
| Native GLSL fallback capability | 7 | Built-in effects/shaders require the WebGL fallback host, which headless rendering/export does not provide. Generated glow and shadow fixtures reproduce the refusal. This needs broader renderer/host work, not extension deferral or removal of the guard. |
| Rotated-video capability guard | 1 | The renderer refuses unverified display rotation. Generated rotated media reproduces the refusal. Correct rendering and reference comparisons require broader renderer work. |
| Original media unresolved | 5 | Four entries have dependent stages blocked by capture limitations; one reports offline originals without a capture error. Missing media and migration/catalog bindings still need separation. The generated missing-original fixture reproduces the guard; these are not established decoder defects. |

Additional capture evidence overlaps these classes: one GLSL case has an unresolved snapshot-payload copy. `ce45a90483a44298` passes all sampled stages but has a dataless associated-input warning, so it is not a clean capture pass. The gate validator refuses capture warnings even when every sampled stage passes.

The earlier 120-second deadline case `a473e29fabf99677` passed all nine stages with the 240-second worker limit. No worker timeout remains in this matrix. Latency SLOs were not measured.

## Fixes and regression checks

- Fixed silent text loss in windowless export children by registering the wake callback for the font broker that the host already pumps. Before the fix, a synthetic text export completed with zero bright glyph pixels; the regression now requires visible glyphs and passes.
- Added deferred, verified IndexedDB payload capture to the migration reader. It snapshots metadata/identities and copies only requested payloads, rejects changed inputs, protects existing destinations, and remains explicitly incomplete for full migration. Existing complete-snapshot identities and normal migration behavior are preserved.
- The runner uses the actual app MCP adapter, accepts kernel safe-copy decisions in private copies, supports legacy composition selection, and distinguishes native built-in definitions from extension dependencies. Restart diagnostics replace stale logs. Disk stops retain completed stage evidence; dataless sources are refused before payload reads.

**347 passing checks:** 275 kernel, 17 first-launch, 27 migration, 11 app-MCP/corpus-worker, 4 export-child and 13 driver safety/scope tests. The legacy visible-text fixture passes all nine stages with the frozen worker. Generated rotated-video, offline-original, glow and shadow fixtures reproduce their expected refusals without owner content.

```sh
nice -n 20 python3 tools/corpus/synthetic.py --worker-only --legacy --text
nice -n 20 python3 tools/corpus/synthetic.py --rotated
nice -n 20 python3 tools/corpus/synthetic.py --offline
nice -n 20 python3 tools/corpus/synthetic.py --effect glow
nice -n 20 python3 tools/corpus/synthetic.py --effect shadow
```

## Method, provenance and limits

The repeatable runner is [tools/corpus](../../tools/corpus/README.md). It uses pm-kernel, windowless Metal pm-engine-service transport, offline audio mixing, actual MCP commands, a temporary PMV round trip, service-child exports and fatal-error FFmpeg decoding. Samples are start/midpoint/90%, eight cached/random seeks, and 0.1-second H.264 MP4/ProRes MOV exports with audio enabled. Output is 320×180. Physical scanout, live speaker/A-V sync, full-timeline visual equivalence and latency SLOs remain unverified.

All native observations use one frozen worker (`7757f8b5e6c0acf`). Driver reporting was refined between disk-limited resumes, so the consolidated summary is explicitly marked mixed-provenance and cannot certify P1. No pre-fix native observations are included. A filesystem search timed out; indexed fallback found additional entries, but the absence of unindexed files is unproven.

Each project was processed in a separate input batch capped at 1.5 GiB; the largest admitted input batch was 984,951,899 bytes (under 0.92 GiB). Payload/checkpoint/save/export reserves and one-second child checks enforce the owner's 6 GiB stop threshold. Shared free space crossed that threshold during the audit; the runner stopped and resumed after recovery. Completed batch media was deleted immediately. Originals were never opened for writing; all writable sessions and exports used temporary copies. No GUI app was launched.

**The private `/tmp/pm-corpus` tree was removed after extracting anonymous evidence.** No owner project/media/name/path or private diagnostic is committed.

## Anonymous evidence

Passed all nine sampled stages (the capture warning above still applies to `ce45a90483a44298`):

`00a141d3279c0ae3` `0652bac842a7e562` `0748a0f955d574ed` `1ce220b50750a14e`
`3eadf609d779dfdb` `4bf47fe5c4208c6e` `4c7fc195f8a3513a` `55ccd73e3d095c91`
`64a30ab374efc5cc` `67f4b158fb98f6d4` `86874fe8ecf3405a` `898d29b5b621180b`
`8fa13c69ae27ff86` `a473e29fabf99677` `b203a840fa18d228` `bae5c0ac2b9d6fb0`
`c308dd126e4a6c8c` `c564920f799d16dd` `c73490ac7186e5eb` `c9a90c4641a685ed`
`ce45a90483a44298` `cfb895e3e7e24a41` `e5b6cf8212e43e74` `f42d35f25012a069`
`fba782b82c84cf07` `fde0e243a4b1f30e`

Other candidates:

| ID | Failure / blocker classes |
|---|---|
| `0a2681304224e9ac` | source_not_local |
| `0aaf78b5ca8c8d3a` | source_not_local |
| `12d2038ab45930f1` | source_not_local |
| `2558fbe470ad20de` | offline_media, source_not_local |
| `26449ed9778ccc7b` | rotated_video_capability |
| `329aa4334946090c` | source_not_local |
| `4005c00c421c75ef` | offline_media, source_unavailable |
| `51f7a2405de3fc96` | source_not_local |
| `5b3631592fb0e66e` | source_not_local |
| `779c9324da71a72d` | glsl_fallback_capability |
| `792b57e8cb4d6f87` | source_not_local |
| `7dbb31affaf298d6` | source_not_local |
| `8018e392aa13c381` | glsl_fallback_capability |
| `899db5132a0d80a5` | glsl_fallback_capability |
| `8af52e9d6db4a6a2` | source_not_local |
| `8e954f016b3ee78f` | source_not_local |
| `9501b730bc7ef8d2` | glsl_fallback_capability, snapshot_blob_changed_or_unavailable |
| `99ebfb70a042321f` | source_not_local |
| `9d4d9482e8484ebb` | glsl_fallback_capability |
| `9e777ba588db52f6` | source_not_local |
| `a63eb19e9a18ef56` | source_not_local |
| `ab3e535075c3b8c7` | offline_media, source_unavailable |
| `c64d523347c50f49` | source_not_local |
| `c65ac925907eba9b` | source_not_local |
| `cc165f3b87a3263c` | offline_media, source_unavailable |
| `d39a085b7fc00d8d` | offline_media |
| `d3a26fa242d9270e` | source_not_local |
| `e194113606334042` | source_not_local |
| `e5a886f3a988c8ef` | glsl_fallback_capability |
| `ee20826fe4919b91` | source_unavailable |
| `f9670f2ae7843523` | glsl_fallback_capability |
| `fab5a1847f8dcc1f` | source_not_local |

Deferred to P2 (confirmed runtime extension requirements):

`06d9cb71af1d93ea` `0b636c1219d7f354` `0eb1bacb2de899e7` `12dde1ae02c148b0`
`1ba16284991c6d1b` `21311a9a65c9e44e` `244bc542b57aaf4e` `2903aef17fda4c83`
`2d3ba78030c652ac` `2dcacbd9b9e550fa` `2f031af05008dc89` `36cf293085f97c18`
`3da4da80adb180d7` `4197bffe17a350a9` `4808611d4e6b2069` `4b27dd91c11e0ef9`
`556c093bd1d655da` `5badd727c40a7461` `63cf45f1a36ce004` `64cf1cf0891cbb8b`
`66b9473572987090` `786f690ab3461cc5` `874491216e92347f` `9b3cc17e5d284715`
`aa5512613386b7db` `af75efc409da84b4` `b376238707965c44` `b402422bfe1a534a`
`b5077b4cf81c35b3` `b5260deb7dbf1c0c` `b732924ad02e30cd` `f739fcc18243c21c`
`f776aad5619ac3f7` `f897aceddf1d277b`

## 2026-11-04 — cloud synthetic follow-up (no new private-project results)

The cloud lane adds portable compiler and journal/media reductions, plus a
one-command frozen-pair Mac preflight/rerun. See the [dated handoff](handoff/corpus-p1.md#2026-11-04--cloud-corpus2-synthetic-reductions)
for exact commands, decisions and limits. **The historical tables above are
unchanged; P1 remains open.**

Measured here: **20/20 public builtin shaders** translate through a real
relocated helper and emit MSL after the original executable copies are removed;
seven representative shader families and real compiler-error attribution pass.
This corrects the historical assumption that every GLSL failure necessarily
requires a WebGL host; it does not resolve any private entry or prove Metal output.

Journal-applied synthetic media reductions expose and fix corpus selection/binding
errors: missed 3D texture-map references, unordered content/bare-ID candidates,
unselected direct content objects, ignored record proxy flags and partial-checksum
ambiguity. Missing originals, changed content and ambiguous provenance are refused.
No private-entry media repair or app-wide Media Store binding is claimed.

Generated FFmpeg rotation references pass exact permutations for all three quarter
turns. Native default-path pixel qualification is prepared, but **the rotation
guard remains** pending Mac proof. Native glow/shadow preflight requires nonblank
preview and decoded MP4/ProRes pixel changes against an effect-free baseline;
it is **not executed or compiled here**, because the native worker requires Apple
frameworks. A Linux example check explicitly fails at `objc2` platform admission.

Linux checks: **112 passed, 0 failed, 1 existing manual private-store test ignored**
across Python driver/frozen-tool tests, Rust corpus reductions, pm-glsl and targeted
kernel library compatibility tests. Portable corpus clippy (`-D warnings`) and
format checks pass. These are synthetic/component results, not additional corpus
passes or a macOS behavior certification. No real projects, paths or media entered
Git; no product design decisions were made.
