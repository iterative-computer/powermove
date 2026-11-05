# WIP: macOS engine and export handoff to Jude

2026-11-01, base Apple M5. Work stopped at the orchestrator's handoff request.
This branch is **not an all-green engine qualification**. The remaining replay,
strict cold-audio deadline and corpus qualification are unfinished. No numerical
parity bound, audio deadline or pending-corpus fingerprint was relaxed.

## Merged scope

Branch: `feat/na-etrack-integrate`.

- `feat/na-cloud-etrack` is in the native-engine ancestry (`4754333b7`), including
  Jude's L4 composition, L5 stills, Phase 4a, big-render and SVG integrations and
  the E-track items. The earlier merge record is in
  [etrack-integration.md](../../engine/etrack-integration.md).
- `feat/na-cloud-promedia2` is also in that ancestry (`80eb437e0`). The
  orchestrator's partial post-merge fixes were parked at `e8716cf23`.
- This lane merged `origin/feat/native-engine` into the parked integration
  branch (`f7722b6b8`), then merged `origin/feat/na-cloud-export` (`bcc5d4034`).
- This WIP adds the Mac repairs and evidence below. It does not finish the
  separate native resource/viewer bindings or qualify every Gate E item.

## Verified and unfinished validation

- The initial combined engine build ran no tests: three Studio API/type errors
  blocked compilation. The first complete engine replay subsequently exposed
  18 failing targets; its per-crate totals are below.
- Latest complete library replay of all 15 requested engine crates: **908
  passed, 0 failed, 2 existing ignored**. Integration targets were being replayed
  separately with all 15 package flags to limit peak link storage. That sweep
  recorded 69 of 140 planned target batches, including interrupted attempts;
  it is **incomplete**. Its earlier
  `libraries` compile failure was superseded by the 908-pass replay, and its
  `output_origin` failure was superseded by a 19-pass targeted replay.
- Latest native replay on the current production source: **pm-native-app
  28 passed, 1 ignored; pm-promedia 116 passed**. `cargo check -p pm-native-app`
  passed. Existing `block v0.1.6` future-incompatibility warning remains.
- Extra engine replay: **pm-node 254 passed, 21 existing ignored; pm-vector
  73 passed**.
- The first complete default-feature Mac export replay: **139 passed,
  7 existing ignored**. The three reported export failures reproduced as
  32 passed / 3 failed across their suites, then passed after repair.
  `pm-export --no-default-features` later passed **69 tests**.
- Seven-crate `clippy --all-targets -- -D warnings` passed. The later change to
  isolate `output_origin` fixtures was verified by its 19 passing tests; the
  entire Clippy command was not repeated after that final fixture-only change.
- The strict cold distinct-voice playback test still fails: the optimized,
  isolated replay was **45 passed / 1 failed**. A recorded first-buffer result
  was 11.053 ms against the unchanged 10.666667 ms limit. **AUDIO-2 remains open**.

First complete engine replay (after the first compile/export repairs),
`passed / failed / ignored`; these are baseline totals, **not final totals**:

| Crate | Baseline P/F/I |
|---|---:|
| pm-audio | 241 / 5 / 1 |
| pm-cache | 3 / 0 / 0 |
| pm-color | 63 / 0 / 0 |
| pm-eval | 77 / 0 / 0, plus failing harnessless `color_ink` |
| pm-export | 139 / 0 / 7 |
| pm-expr | 34 / 0 / 0 |
| pm-geom3d | 66 / 0 / 1 |
| pm-glsl | 70 / 0 / 0 |
| pm-gpu | 33 / 1 / 3 |
| pm-media | 134 / 0 / 31 |
| pm-protocol | 119 / 0 / 1 |
| pm-proxy | 21 / 1 / 0 |
| pm-reference | 51 / 0 / 0 |
| pm-render | 1133 / 27 / 9 |
| pm-wasm | 42 / 0 / 0 |

## Failure register

Names below are exact Rust test names unless explicitly identified as a
harnessless target or compile/check failure. “Fixed” means a relevant replay
passed; it does not mean the interrupted full integration sweep finished.

### Build and check failures

| Failure / target | Cause and local repair | Status |
|---|---|---|
| `pm-render` Studio compilation: missing `Rendered3d::color_space`, removed `GeometryPayload::groups`, removed `Materials::Slots` | Adapt to the current whole-mesh/single-material API. Label Studio's already tone-mapped/sRGB output `Encoded`. Add material/output regressions. | Fixed; engine libraries and native tests/check pass. |
| `pm-proxy --test chart`, `pm-proxy --test sequence`: missing `mod color` source | Tests imported the deleted S2 precision spike. Retain only the needed independent f64 sRGB/XYZ/Lab/CIEDE2000 reference under `tests/support/color.rs`, including Sharma vectors. | Fixed. |
| `pm-export` tests: missing `convert_rgba8_into` | Merge removed a helper still consumed by tests. Restore it through the common dimension/fill conversion; reject HDR before destination mutation. | Fixed; export replay passes. |
| `apps/native` locked dependency graph | Four local dependency edges missing from `Cargo.lock`; no dependency version bump needed. | Fixed; locked native replay passes. Engine lockfile did not need a change. |
| `pm-render --lib`: two missing `convert_format` arguments in `media/source_storage_tests.rs` | Tests needed the new still-versus-video chroma-selection argument. Preserve the video reference path. | Fixed; 177 render library tests pass. |
| `pm-export --no-default-features --lib`: `install_qos_refresh` unavailable | Installer/guard were gated to the engine feature although Mac row-conversion tests need them. Admit them under `test` as well. | Fixed; 69-test encoding-core replay passes. |
| All-target Clippy | Unused imports; nested conditionals; callback/fixture tuple type complexity; test modules before production items; manual `Textures::default`; SVG reference file loaded twice. | Fixed with scoped import cleanup, equivalent conditionals/type aliases/derived default, moved test modules and one shared reference module. No blanket warning suppression. |
| Protocol regeneration (`wire::typescript_regeneration_is_a_no_op`, known incoming ledger failure) | Generated desktop protocol id lagged the Rust source hash. Apply the prepared generated-id update to `protocol-f1801e99aec59477`. | Fixed; strict Mac protocol tests pass. |

### Export and pro-media

| Test | Cause and local repair | Status |
|---|---|---|
| `all_audio_codings_preserve_rate_channels_length_and_tone` | AAC's final transform/flush block was incorrectly included in the steady-state metric. Exclude both 1024-frame ends only from that metric; keep full-program RMS, length, rate and channel bounds. | Fixed. |
| `nothing_appears_at_the_destination_unless_the_export_completes` | Successful atomic replacement also retains the old file in the archive. Assert the delivered file and archive, and check the original archived bytes. | Fixed. |
| `sdr_color_bar_chart_through_h264_hevc10_and_prores` | The Mac AVFoundation CPU reference reader's packed-BGRA conversion/encoder constraints were unsuitable. Read macOS ARGB without Metal/IOSurface encoder constraints and reorder to BGRA. `PixelBuffer::bytes` now uses the actual CoreVideo constants, including numeric ARGB. | Fixed; all original color bounds remain. |
| `unverifiable_variable_duration_is_refused_without_inventing_timing` | Mac ffprobe reports explicit 40-unit durations at variable PTS 0, 40, 160, 360. Acceptance is correct; the fixture did not establish missing duration. Replace the version-dependent assumption with deterministic absent/N/A/zero final-duration probe records; add exact reported-VFR digest coverage. | Fixed; probe production logic unchanged, native suite passes. |

### Audio

The independent headless device fixtures share a process-wide worker pool,
idle leases and diagnostics. Concurrent tests could charge another simulated
device's scheduling state to the current test. All 46 playback fixtures now
hold one process-wide device-fixture mutex. Test-profile engine code is
optimized (`opt-level = 2`), with assertions and overflow checks retained.

| Test | Cause / outcome | Status |
|---|---|---|
| `a_scheduled_play_is_heard_on_the_line_that_starts_at_its_time` | Shared fixture scheduling interference; original timing/PCM assertions retained. | Fixed in targeted replay. |
| `edit_then_play_reuses_all_sixteen_stretched_voices` | Shared device/idle-lease diagnostics interfered. | Fixed in 45/1 playback replay. |
| `edit_then_play_reuses_sixteen_distinct_stretched_voices` | Same fixture ownership issue. | Fixed in 45/1 playback replay. |
| `starved_playback_recovers_without_poisoning_dsp` | Same fixture ownership/scheduling interference. | Fixed in targeted replay. |
| `cold_seek_and_speed_edit_prepare_sixteen_voices_in_parallel` | Failed an earlier partial/unoptimized playback replay; passes in the later optimized isolated replay. | Fixed in targeted replay; full final replay not reached. |
| `cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous` | Real cold first-512-frame deadline misses remain after fixture isolation and optimization. | **Open, AUDIO-2**. No threshold change or ignore. Worker-count and shorter-priming prototypes were tested and reverted; no DSP/pool/priming runtime change remains. |

### Images, media and color

| Test / target | Cause and local repair | Status |
|---|---|---|
| `pm-eval --test color_ink` (harnessless `main`) | Frozen metrics describe Helvetica; the test requested optional SF Pro Display, which resolves differently on this Mac. Make the synthetic fixture explicitly Helvetica and keep every frozen number. | Fixed; harnessless replay passes. |
| `still_images` (`pm-gpu --test decode_import`) | Test assumed all stills have one RGB plane. AVIF now retains native YCbCr planes. Use the shared independent normalization comparison; retain the 1-ULP bound and existing video zero-copy checks. | Fixed; final decode-import target passes. |
| `real::sequence_color_tags_follow_p3_and_preserve_exr_highlights_and_other_icc` | New macOS/sips PNGs carry cICP as well as ICC; PNG cICP overrides ICC. Honor supported explicit primaries/transfer, preserve AVIF ICC precedence and reject unknown code points. | Fixed; four-test sequence replay passes. |
| `stills::tests::static_formats` | Stale support list excluded JPEG/AVIF/EXR now read by the native path. Update the contract assertion. | Fixed; library replay passes. |
| `original_sdr_heif_depth_alpha_orientation_and_profile_reach_source_frames` | New ImageIO can produce precision-preserving half-float SDR. Preserve the independent Swift ImageIO/ColorSync reference instead of requiring integer-only layout. Renamed to `original_sdr_heif_precision_alpha_orientation_and_profile_reach_source_frames`. | Fixed; renamed final test passes. |
| `minified_stills_sample_their_own_mip_chains` | Mip filling ran on a one-level source texture. Allocate a real mip chain. | Fixed in targeted replay. |
| `native_source_gamma_png_preserves_file_curve_depth_and_partial_alpha` | Stale reference treated gAMA=45455 as piecewise sRGB. Preserve PNG's declared pure-power curve and partial alpha; calculate the reference independently. | Fixed in targeted replay. |
| `h273_log_png_tags_decode_in_float_at_every_sample` | Admission rejected native H.273 gamma/linear/log curves that the reader implements. Admit supported full-range RGB codes and retain unknown-code refusal. | Fixed in stills replay. |
| `avif_color_descriptions_blink_reads_as_srgb_are_drawn` | Stale assertion refused now-supported AVIF gamma/linear descriptions. Assert admitted, distinct output; retain rejection of unknown descriptions. | Fixed in stills replay. |
| `stills_are_at_least_as_close_to_the_reference_as_today` | Test requested Float32 but left the authoritative project depth at 16. Set/assert actual 32-bit output. AVIF still chroma needed normalized Lanczos-3 reconstruction; video keeps its original kernel. Low-bit packed RGBA normalization used 255 instead of declared precision. | Fixed; 17-test standards replay passes with original bounds over the supplied still fixtures. |
| `image::imageio::sample_tests::unprofiled_tiff_palette_entries_are_not_color_matched` | Initial regression fixture exposed ImageIO color-table rounding. Expand packed indices through the original bounded TIFF tag-320 16-bit RGB table. | Fixed; all three new ImageIO sample tests pass. |
| `additional_integer_decoder_classes_are_drawn` | Palette comparison used FFmpeg's rescaled RGB8 conversion rather than the original TIFF entries. Reader repair and test now preserve the stored 3:3:2 table; helper also handles native 16/float layouts. | **Partly fixed**: authored palette unit tests pass; the whole integration test was not rerun successfully after the last palette repair. Prior failing attempts remain relevant. |
| `offline_and_corrupt_media_are_named_failures_without_routing` | Stale compatibility-level split for corrupt/empty SVG after the native SVG route was unified. Apply the current broken-image/empty contract at both levels, retaining other named media failures. | Fixed; final media-sequences replay passes. |

### 3D, shader and fixture ownership

| Test(s) | Cause and local repair | Status |
|---|---|---|
| `maps_that_change_after_planning_route_or_drop`; `short_uv_model_draws_when_override_map_fails_after_planning` | Standalone 3D render entry point no longer prepared native maps. Add a lazy native media/pipeline cache; invalidate resolved scene data when a new map arrives. | Fixed in targeted replay. |
| `maps_with_transparency_draw_and_missing_maps_draw_without_them` | Stale expectation darkened straight-alpha color-map samples. Keep the native straight-color contract and existing map/missing-map assertions. | Fixed in targeted replay. |
| `corrupt_legacy_scene_checkerboard_carries_its_missing_metadata` | Pipeline/definition preparation needed the correct same-checkout compiler helper. | Fixed in targeted replay. |
| `uniform_binding_follows_the_compositor_quirks`; `time_frame_mouse_and_resolution_uniforms`; `extension_fragment_layers_bind_their_parameters`; `level2_shader_layers_render_at_the_density_they_are_drawn_at`; `missing_extension_definitions_draw_the_checkerboard_and_unknown_ones_route`; `targets_are_capped_at_4096_pixels`; `code_webgl_refuses_draws_nothing_and_is_reported` | Independent fixtures raced for the intentionally two-slot process-wide compiler; an old helper could also invalidate preparation. Serialize these success fixtures and use the current helper. Production admission and separate contention/recovery tests stay unchanged. | Fixed; all 10 shader-layer tests pass. |
| `shader_layers::exec::tests::a_pipeline_the_device_refuses_goes_to_the_runner_on_first_contact` | Render-library success fixtures also competed for compiler admission. Share a test-only mutex across the five relevant library fixtures. | Fixed; 177 render library tests pass. |
| `a_disabled_effect_stays_off_across_unrelated_pushes_and_other_views` (`pm-node --test definitions`) | Fixture created an unused second renderer merely to borrow a device. Construct the GPU directly so Composer owns the sole renderer. | Fixed. |
| `agent_frames_and_exports_draw_the_definitions_their_session_holds`; `pushed_definitions_are_compiled_before_they_are_acknowledged` (`pm-node --test jobs`) | Independent fake hosts competed for compiler admission and reported unrelated fallback definitions/output. Hold the fixture mutex for each fake host's lifetime. | Fixed; all 13 jobs tests and the complete 254-test node replay pass. |
| `an_integer_origin_draws_the_part_of_the_whole_frame_it_starts_at`; `a_fractional_origin_moves_every_layer_like_a_composition_translation` | Reproduced again late in the broad sweep: four output-origin fixtures concurrently installed builtins into the two-slot compiler. Isolate all four fixtures. | Fixed; latest targeted replay **19 passed / 0 failed**. |

### Required external parity gates

| Test / target | Cause / evidence | Status |
|---|---|---|
| `path_mask_coverage_gate` | No original mask witness on this Mac. Fresh 54-case M5 capture and Metal/f64 gate pass at unchanged bounds. | Partly fixed: component execution verified against new witnesses; original frozen qualification remains with Jude. |
| `shapes_and_paths_meet_the_reference` | No original Canvas witness. Fresh 556-case capture, 474 shape/path comparisons and 18-test vector gate pass. | Partly fixed: fresh witness verified, original qualification pending. |
| `svg_images_meet_the_reference_and_todays_interiors`; `svg_sharpness_under_scale` | No original SVG frame manifest. Fresh 43 actual legacy-app frames pass the placement/scale gates and their source/frame-hash checks. | Partly fixed: fresh-witness replay passes 8 tests; original qualification pending. |
| `svg_probe_documents_against_chromium` | No original semantic witness. Fresh 383 Chromium captures, 374 compared rasters pass; existing intrinsic-size/radius decisions retained. | Partly fixed: fresh witness passes, original qualification pending. |
| `corpus_render_parity`; `corpus_own_layers_reference_gate`; `corpus_plans_with_reused_draws_match` | Unfiltered default-corpus replays fail because the original capture folder is absent. Fresh category runs additionally reveal reference gaps, changed/resolved pending measurements, unsupported paths and actual parity failures. | **Open**; use Jude's corpus before qualifying or changing the pending ledger. |
| `pm-eval --test corpus` (harnessless `main`) | Fresh text category leaves 6 SF Pro layouts outside the comparison, above their zero pin. Other category evaluation comparisons also need the original witness. | **Open**; no font-exclusion pin increase. |
| `pm-render --test text_corpus` (harnessless `main`) | Fresh layers/compositing/mattes/retiming/space categories include unowned-reference or pending-fingerprint failures. Dedicated text checks pass for several other categories. | **Open**; no pending fingerprint replacement. |

## Corpus and Gate E ownership

The frozen corpus and real project were not present on this Mac. The captures
used here are **fresh synthetic v1 captures on the build Mac (base M5), not the
original frozen capture**. They used the documented tool, `--skip-real`, a
temporary Electron profile and `/private/tmp/pm-engine-corpus`. Categories were
checked and removed after recording results. The sweep was interrupted; it is
not a full-corpus qualification. A final capture attempt was resource-stopped.

Media, shapes, animation, composition, masks and backgrounds passed their
completed category render/text checks. Several other categories failed. In
particular the fresh render category compared 135 frames and reported 223
failing checks, including changed/resolved pending entries; these are not 223
independent proven rendering defects. The 3D category compared 43 frames with
19 reported failing checks, and composite compared 29 with 57. Text's combined
corpus command initially stopped at its failing evaluator before the renderer;
later category commands used `--no-fail-fast`. Rerun it fully with Jude's data.

Jude's corpus/projects are needed for:

1. Full evaluator/render/own-layer/text/draw-reuse corpus gates and their pinned
   pending measurements; original font/layout qualification and real-project
   end-to-end export/evaluation.
2. Original mask, Canvas and SVG witnesses when asserting the original frozen
   qualification rather than the new M5 component checks above.
3. Original WebGL pixels for S4 comparisons and its known fallback-only gaps.
   The native M5 inventory did run: 812 sources, 2,143 variants, 656 Metal draws;
   the remaining cases stopped at validation/binding/limits/Naga, including
   deliberately invalid probes. This does not close the historical pixel gaps.

**Gate E remains OPEN**: the static audit accounts for 17 categories and 74
production references, with 15 unqualified categories and 9 open work items.
Corpus availability alone also does not establish missing native resource
bindings, physical output/listening routes, 60/120 Hz performance or long runs.
See [the Gate E ledger](../../engine/e2e-ledger.md) and
[gate-e.json](../../engine/gate-e.json).

## Environment and reproduction commands

Run from this checkout on macOS. Python 3.9 is insufficient for pm-media's
native build; use the explicitly selected Python 3.14. Canonical private temp
paths avoid `/var` versus `/private/var` safe-path test failures.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_BUILD_JOBS=4
export PM_MEDIA_NATIVE_PYTHON=/opt/homebrew/bin/python3.14
export TMPDIR=/private/tmp/pm-test-tmp/
mkdir -p "$TMPDIR"
```

Every build/test ran one at a time under `/tmp/pm-build.lock`, at `nice -n 20`.
Measure storage with `diskutil info /`'s **Container Free Space**; the floor is
5 GB (5,000,000,000 bytes). Do not use `df` alone for this floor. Honor the Mac's
`PAUSE_LOCAL`/`USER_ACTIVE` signals before launching apps/captures.

A shared-target artifact was observed being reused from another checkout.
Validation salted Cargo's keys with an identity remap, which changes no paths
or program semantics. In a clean shell with no other Rust flags, the equivalent
setting from the repository root is:

```sh
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$PWD=$PWD"
```

Build the GLSL helper from this same checkout and use that executable (a private
copy avoids another worktree replacing the shared output basename):

```sh
(cd engine && nice -n 20 flock /tmp/pm-build.lock \
  cargo build --locked -p pm-glsl --bin pm-glsl-compiler)
export POWERMOVE_GLSL_COMPILER="$CARGO_TARGET_DIR/debug/pm-glsl-compiler"
```

With Jude's full capture and auxiliary witnesses selected, run the requested
unfiltered command. `PM_ENGINE_CORPUS` names the full project capture;
`PM_ENGINE_CORPUS_ROOT` names the parent of the `masks-*`, `canvas-*`,
`svg-gate-*` and `svg-frames-*` capture folders. Explicit `PM_MASK_REF` and
`PM_SVG_FRAME_REF` can select those component captures.

```sh
cd engine
nice -n 20 flock /tmp/pm-build.lock cargo test --locked --no-fail-fast \
  -p pm-audio -p pm-cache -p pm-color -p pm-eval -p pm-export -p pm-expr \
  -p pm-geom3d -p pm-glsl -p pm-gpu -p pm-media -p pm-protocol -p pm-proxy \
  -p pm-reference -p pm-render -p pm-wasm
nice -n 20 flock /tmp/pm-build.lock cargo test --locked --no-fail-fast \
  -p pm-node -p pm-vector
nice -n 20 flock /tmp/pm-build.lock cargo test --locked \
  -p pm-export --no-default-features
nice -n 20 flock /tmp/pm-build.lock cargo clippy --locked --all-targets \
  -p pm-media -p pm-gpu -p pm-render -p pm-export -p pm-audio -p pm-node \
  -p pm-proxy -- -D warnings
cd ../apps/native
nice -n 20 flock /tmp/pm-build.lock cargo test --locked \
  -p pm-promedia -p pm-native-app
nice -n 20 flock /tmp/pm-build.lock cargo check -p pm-native-app
```

Fresh synthetic recapture, if needed, remains the documented procedure below.
The capture tool supplies the temporary Electron profile. Choose one category,
check it, save results, then remove its capture before the next category.
Never relabel it as the original frozen witness.

```sh
unset ELECTRON_RUN_AS_NODE
nice -n 20 flock /tmp/pm-build.lock flock /tmp/pm-electron.lock \
  bun engine/corpus/src/capture.ts --label v1 --id v1-local-layers \
  --only layers/ --skip-real --skip-scenarios --root /private/tmp/pm-engine-corpus
```

## Next steps

1. Review this WIP against Jude's branch/corpus, especially Studio whole-mesh
   semantics, PNG cICP/palette preservation, still chroma reconstruction and
   the explicit Helvetica fixture. Complete the unfiltered engine replay;
   final all-crate integration totals have not been established.
2. Rerun `additional_integer_decoder_classes_are_drawn` after the last TIFF
   repair. Rerun all-target Clippy after the final output-origin fixture guard.
3. Fix the real cold distinct-voice AUDIO-2 deadline without changing PCM,
   priming/history correctness, timing bounds or stress assertions.
4. Run the corpus gates with the original frozen captures and test projects.
   Classify fresh-Mac differences against that evidence. Do not blanket-bless
   changed/resolved fingerprints or broaden exclusions to make a gate pass.
5. Complete the remaining native/physical/performance Gate E work separately;
   keep the ledger open until those proofs exist.

No new builds, tests, captures or app launches should be inferred from this
handoff. The implementation lane stopped its jobs and is handing off the WIP.


## 2026-11-04 — cloud etrack2 follow-up (Linux; still BLOCKED)

Integration and current native-engine histories merge without conflicts. The
latest merged base is `711f44ba0667f81ce8388c4ec57565d1d0378630`. Exact commands,
counts, picking rules, process exceptions and per-item corpus dependencies are
in [cloud-etrack2.md](../../engine/cloud-etrack2.md).

- Guarded vocoder calls now batch output-only spans between synthesis intervals.
  Original scalar processing remains the bitwise reference. Pre-roll, DSP calls,
  source positions, head/lead sizes and the 10.666667 ms deadline are unchanged.
  A portable harness exercises the actual canonical run with explicit scheduling/
  profiling test doubles. Its source reads and every sample bit match for sixteen
  distinct trims and both directions. This is not simultaneous device playback.
- The available, unfiltered 12-package replay with `--no-fail-fast` finished
  **661 ordinary tests passed / 0 ordinary tests failed / 3 inherited ignored**, plus
  **one failed executable target**: `pm-eval --test corpus`, now explicitly blocked
  by macOS CoreText/frozen witnesses rather than a Linux compile failure or a
  successful missing-capture return. Geometry's actual digest failure was repaired
  by canonicalizing only unrecorded NaN payload/sign bits in the test/generator.
  Finite coordinates, signed zero, fixture bytes/hashes and all bounds stay exact.
- Generation-disabled proxy: **9 passed**. Engine-disabled export: **67 passed**,
  including 14 software-delivery cases; no codec skips. Coordinate-generator
  helper: **4 passed**. All-target clippy passes for all 12 portable packages and
  generation-disabled proxy; the three touched crates are format-clean.
- The requested full 15-crate engine replay, node/vector replay, exact TIFF target,
  exact cold playback target and full 18-package all-target clippy were attempted
  again after the final base merge. They stop before execution at Mac media/Apple
  dependencies. **No successful TIFF integration rerun or AUDIO-2 pass is claimed.**

The original **11.053 ms versus 10.666667 ms** failure remains open. Run plain
and stressed Mac playback campaigns with profiling disabled, preserving complete
PCM, simultaneous starts, scheduling and zero-underrun assertions. Separately
rerun the TIFF integration target after the historical palette repair, then the
complete engine replay and all-target clippy with the native SDK/library setup.

Gate E stays **OPEN: 15 unqualified categories / 9 open work items**. Original
frozen evaluation/render/text/own-layer/reused-draw, font/layout, mask/Canvas/SVG,
WebGL and project/export witnesses remain required. Missing native bindings,
resource/sequence support, audio decoder/export semantics, CHROMA-420-1 and
physical/performance gates are independent remaining work. The newly merged P0
owner decisions also require latest-main acceptance and the separate new-3D port;
existing witnesses are not relabelled as proof for those requirements.


## 2026-11-04 — final base refresh

Final merged native-engine base: `78294c4c93cdb2a3a18aef8004fc42f9e5e420a9`, merged without
conflicts in `9d8acb18e`. This update contains design documentation only; its
engine tree is identical to the preceding base. The staged secret scan ran
before this merge commit, which includes the requested co-author trailer.

All ten Cargo replay/clippy commands listed in the cloud etrack2 report were
repeated after this merge. Results remain **661 ordinary passing tests / zero
ordinary failures / three inherited ignores, plus the one explicitly failed
CoreText/corpus executable**; proxy **9/9**, export core **67/67**, portable
all-target clippy clean. The full engine/node/TIFF/cold-audio/full-clippy
commands remain build-blocked before execution by Mac media/Apple dependencies.
Node helper tests **4/4**, Python helpers **5+5**, standalone device policy
**5/5**, formatting and the static inventory were also checked again. No new
Mac, frozen-corpus, latest-main or new-3D qualification is inferred.
