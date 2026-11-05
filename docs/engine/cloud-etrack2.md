# Engine integration: cloud etrack2

## 2026-11-04 — scope and decisions

Base inputs: integration `7ca9048a586c824fcbfa138f4ae93285b07a7087`
and native-engine `ed5482622f41c533199a724dcb8c8738dd4dcc93`.
The two histories merge without conflicts in `327261794`.
The original [Mac handoff](../native/handoff/engine-macos.md) remains historical
proof, including its **11.053 ms** cold-first-buffer failure against
**10.666667 ms**. This Linux work does not supersede that failure.

| Decision | Picking rule | Consequence |
| --- | --- | --- |
| Batch only vocoder output between completed synthesis steps; retain the original scalar specialization for unguarded calls | N1 (less real-time work), N3 (authored timing and exact PCM) | Removes repeated sample-loop dispatch and per-sample output-ring indexing/clearing. Native process lengths, synthesis boundaries, input offsets, FFTs, seed, pre-roll, lead/head sizes and scheduling checks remain unchanged. |
| Dispatch the specialization once at the C binding, including internal output-seek and flush pre-roll | N1 | The scalar path acquires no per-sample feature check. Split computation retains its scalar path. |
| Exercise the actual `pm-audio/src/stretch.rs` in a portable harness, with only error/profile/scheduling seams replaced | N3, N7 (inspectable independent evidence) | Source reads and every PCM bit are compared for guarded versus scalar canonical runs; the harness does not claim CoreAudio scheduling, pool or device timing. |
| Retain the strict AUDIO-2 test and original corpus fingerprints unchanged | N2, N3 | A work reduction or Linux pass cannot qualify Mac timing or frozen rendering parity. |
| Preserve both branch histories and retain all existing consumers and witnesses | N4, N6, prune rule | No reference capture, legacy implementation or user data is removed. |

### Audio diagnosis and proof limits

The known expensive phase is applied-to-ready: sixteen independent canonical
vocoder histories must complete before the producer can publish the first PCM.
Distinct trims cannot share a head. The native loop previously revisited its
block/step logic and read/advanced the STFT ring once per output sample even
when the spectrum was complete and the next synthesis interval was hundreds
of samples away. The batch reads the same divisions and clears the same output
samples using the STFT's existing bulk operations. It stops at the next interval
or native call end. All DSP steps and their checkpoints still execute.

This identifies and removes a concrete source of preparation work. It does
**not** identify which portion of the historical 11.053 ms observation was
CPU work, scheduling, contention or queue delay: that record has no matching
per-operator profile in this environment. AUDIO-2 remains open until the actual
Mac playback regression passes, plain and stressed, without a deadline change.

`pm-stretch/tests/stretch.rs` compares the bulk and original scalar histories
at 44.1/48/96 kHz, 1/2/6/32 channels, speeds 0.35/0.5/1.5/1.6/2, two reset
and transpose epochs, zero-input and zero-output calls, partial intervals,
wraps and flushes. The portable canonical harness also imports the existing
run exactness tests (slices, missing-input recovery, silence/reverse class
changes, loop wraps and the proof that shortening pre-roll changes PCM).
Its added matrix compares actual source-read calls and 4,718,592 kept sample
bits per implementation for sixteen distinct trims, three seek positions,
three speeds and both directions. Test scheduling is explicitly simulated.

`pm-stretch/examples/cold_work.rs` is a counterbalanced work probe for scalar
versus batched native construction/seek/process spans. Every pair asserts all
rendered PCM bits. It is sequential and excludes the playback pool, evaluator,
decoder and device; it cannot establish the 512-frame deadline.

## Gate E: exact dependencies

The static inventory still accounts for **17 categories / 74 production
references**, with **15 unqualified categories / 9 open work items**. No
`verified` flag changes on the strength of this Linux run. The following
separates missing frozen witnesses from implementation, specification and
platform work. A missing corpus is not an explanation for the latter.

### Unqualified capability categories

| Category | Work independent of the frozen corpus | Frozen evidence needed for qualification |
| --- | --- | --- |
| LayerType | Run the shipped-type inventory, actual draw and unknown-type diagnostic tests on Metal | Full authored-document render/own-layer/reused-draw corpus; empty unsupported set |
| ExtensionDefinition | Install fragment/mesh definitions; bind the V8 recorder producer and native resources (runtime/service owners outside this lane) | Supplied extension/project pixels and G-E1's original WebGL witnesses |
| Effects | Real definition/EffectHost installation; independent f64 chain and execution-error tests on Metal | G-C2/effect-proof chains and unchanged pending fingerprints |
| Transition | Native program/resource binding and f64 transition tests on Metal | Original transition/project frames and pending fingerprints |
| MotionBlur | Sampled-type coverage, shutter/own-layer math and actual Metal execution | Original shutter/composition/own-layer frame witnesses |
| TwoAndAHalfD | Native plane/projective sampling, picks and bounds tests | Original space/3D/composition frames and pending fingerprints |
| Adjustment | Accumulator/effect-chain execution and definition lifecycle tests | Original affected compositions and pending fingerprints |
| CompatLevel2 | Keep the public variant while consumers exist; native semantics/coverage tests | Original comparison witnesses where applicable, plus independent level-2 semantic gates (not legacy equivalence for a new level) |
| RequiresEngine | Capability/stamping inventory and named errors; no fallback runner | Full corpus with original media/fonts/definitions |
| ProjectColor | Professional project-colour contract, explicit unsupported-mode errors, standards and precision tests | Original project/export comparisons; unknown/future modes also need specifications, not recapture |
| ImageSequence | Original-frame timestamp provider/registration and native animated-format playback, not just metadata or persisted renditions | Original sequence/media project playback and preview/export comparisons |
| Media | Apple decode/normalization, TIFF/palette, HEIF HDR, default-reading geometry, cache invalidation, native codec/reading implementation | Original still/video and project witnesses for G-I/G-C; TIFF regression itself uses synthetic fixtures and needs **Mac, not frozen corpus** |
| Definition | Serialize installed definitions/project fonts to native export child; host lifecycle (outside this lane) | Original definition-dependent project/export witnesses |
| ShaderFallback | Implement missing native sampler/resource types/arrays; Metal compile/draw/refusal inventory | Original WebGL input/parameter/storage perturbation pixels for G-E1; native compile success alone does not close it |
| Text | CoreText/project-font installation, glyph paint errors, semantic/outline tests on Mac | Original text layout/font seeds/census, isolated captures, text-composition and pending fingerprints |

`Masks` and `Request` retain their existing verified status; no new claim is
made for them here. Mask/component frozen qualification remains separate.

### Open work items

| Item | Can proceed without frozen captures | What still needs the original witnesses |
| --- | --- | --- |
| L7 | Canonical DSP work reduction and portable exactness; Mac audio decode/mix/stretch/playback, strict AUDIO-2, allocations, native replacement of Chromium decoder/legacy speed-ignoring export path, listening | Paired shipped-app decode/export/authored project comparisons; no frozen pixels are needed to measure AUDIO-2 itself |
| AUDIO-DEV-1 | Device policy and real stream reopen/reanchor integration; physical route/rate/channel/hotplug/no-device/8192-frame callbacks | None for device lifecycle; real Mac devices are required |
| Log interpretation | H.273 9/10 CPU/WGSL and Mac probe-to-pixel tests; exact proprietary curve **and gamut definitions** before admission | Original source/project comparisons where such media exists; missing proprietary specifications are an independent blocker |
| Standards gaps | CHROMA-420-1 antialiasing/inverse proof; HEIF HDR/source readings; lossless encoder-input f64 bars and source-depth tests | G-I1 and G-X1 paired original captures. Synthetic standards proof does not require the frozen project corpus |
| WebM settings | Native VPx/Opus alpha, sample count, priming, duration, tags; matched-quality Mac throughput | Paired original/shipped export encoding evidence for G-X1; real codec builds and matched hardware also required |
| SHADER-RES-1 | Native sampler arrays/resource bindings and exact logical coordinate/uniform semantics; compile inventory | Original G-E1 WebGL pixels and perturbation witnesses |
| Phase 4a proxies | Portable store/policy/manifest/cache; service GenerateProxy/representation/paused upgrade/cache bindings **outside owned paths**; Mac codecs, relink/relaunch/cancel/disk tests | Original-versus-proxy project/export references and matched performance workloads; implementation binding does not depend on corpus availability |
| Bigrender | Prepared-track/prewarm tests; Mac allocations/texture memory, cancellation/retirement, real playback/scrub/export | Full project/corpus/pending fingerprints and prescribed paired 60/120 Hz workloads |
| Corpus and performance | Complete missing native host endpoints and physical cadence/AV-sync/preview=export proofs | G-C/G-T/G-I/G-E/G-X, full evaluation/render/own-layer/text/reused-draw corpus, original mask/Canvas/SVG witnesses, font/layout qualification, real-project end-to-end export and unchanged pending measurements |

The existing native-service, V8/remote-DOM, UI, physical routes and performance
owners remain necessary. This lane changes no capabilities to optimistic
success and enables no Chromium/Node fallback.

## 2026-11-04 — replay admission repair

The first portable all-target replay could not compile `pm-eval --test corpus`:
it unconditionally imported the macOS-only `CoreTextRegistry`. The harness now
selects its existing implementation only on macOS, under `tests/support` (not
an extra Cargo test target). Linux emits a named **BLOCKED** failure. With
`--no-fail-fast`, unavailable corpus proof no longer prevents the independent
CPU suites from running. On Mac, absent captures also fail explicitly; the
previous successful early return was not corpus proof. Numeric bounds, font
exclusion pins, semantics and capture selection are unchanged. **N3/N7 and the
honesty rule** select this behavior; there is no substitute font or skipped
case. The moved fixture include remains inside the owned engine tree.

## 2026-11-04 — portable geometry witness repair

The unfiltered CPU replay exposed `paths_match_vector_paths_ts`: actual digest
`2735fae539268e85` versus stored `93f367b1fabb1185` for `path1`. The readable
coordinate comparison immediately before the digest already passed with its
existing exact finite-bit comparison and NaN-class equivalence. The remaining
difference was the sign/payload of NaNs from non-finite cubic inputs, which the
JSON witness stores only as `"NaN"`. That payload is not authored geometry.

The Rust witness digest and the generator now explicitly encode every NaN as
IEEE `0x7ff8000000000000`, with little-endian storage. Finite values (including
one-ULP differences), signed zero and signed infinities remain bit-exact and
have regressions on both sides. The generator helper is independently runnable
with Node 22.18+; its tests check every readable frozen coordinate digest.
All existing fixture hashes, fixture bytes, geometry code and numerical bounds
remain unchanged. **N3 and the quirk policy** select a portable fingerprint of
the recorded numeric value instead of reproducing a hardware NaN sign in the
renderer. This is separate from the native/WASM production conformance digest,
which was not changed. Formatting-only changes keep the touched crate clean.

## 2026-11-04 — newer integration base

The final base fetch advanced native-engine to
`711f44ba0667f81ce8388c4ec57565d1d0378630`, merged cleanly as `38f32c453`.
The incoming update is documentation only. It records the owner's latest-main
reference (`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`) and the separate new-3D
port requirement; see [main-delta.md](../native/main-delta.md), especially E01–E18.
The existing v1.2 witnesses and pending ratchets remain intact and required.
No latest-main effect-version/resource, 3D or output qualification follows from
this Linux replay, and no existing witness is silently relabelled as the new
reference. Those requirements add to the remaining integration work.

Process exception: this second merge auto-committed before the staged secret
scan. A direct first-parent merge-commit scan subsequently inspected one commit
and 269,758 bytes with zero findings. Subsequent merges use `--no-commit` so
the staged scan precedes the commit. This exception does not represent the
required pre-commit scan having run retroactively. That automatic merge also omitted the requested
co-author trailer; the already-pushed merge history was retained.

## 2026-11-04 — final Linux validation

All commands below were repeated after merging native-engine
`711f44ba0667f81ce8388c4ec57565d1d0378630`. Rust is pinned to 1.96.1. The
portable codec checks used FFmpeg/ffprobe 7.1.5; their explicit capability
checks reported no skips. These are Linux software-encoding checks, not Mac
VideoToolbox or shipped-library equivalence. No dependency edges or versions
changed, so neither workspace lockfile needs a new update for this lane.

| Replay | Result |
| --- | --- |
| Full 15-package engine replay, unfiltered, `--no-fail-fast` | **BLOCKED**, exit 101 before any tests: pm-media requires its Mac FFmpeg dylib build |
| pm-node + pm-vector replay | **BLOCKED**, exit 101 before any tests: same Mac media dependency (the earlier missing Linux ALSA development metadata was supplied in the task environment) |
| TIFF `additional_integer_decoder_classes_are_drawn` | **BLOCKED**, exit 101 before execution: Apple-only objc2 dependency; no successful TIFF integration rerun is claimed |
| Cold distinct-voice playback | **BLOCKED**, exit 101 before execution: Mac media dependency; historical 11.053 ms failure remains open |
| All-target clippy, 18 packages, `-D warnings` | **BLOCKED**, exit 101: Mac media/Apple dependencies; not an all-green clippy claim |
| Available 12-package CPU replay, unfiltered, `--no-fail-fast` | **661 passed / 0 ordinary test failures / 3 inherited ignored; 1 failed executable target** (`pm-eval --test corpus`, explicit CoreText/frozen-witness blocker). Command exits 101 |
| pm-proxy without native generation | **9 passed / 0 failed / 0 ignored** |
| pm-export without engine integration | **67 passed / 0 failed / 0 ignored**, including 14 software-delivery tests; no codec skips |
| All-target clippy for the 12 portable packages and separate generation-disabled proxy | **Passed**, both with `-D warnings` |
| Node coordinate-witness helper | **4 passed / 0 failed / 0 skipped** |
| Gate inventory and audio campaign Python helpers | **5 + 5 passed** |
| Standalone actual device-policy module | **5 passed**; policy arithmetic only, no stream or physical-device proof |
| Formatting | `pm-eval`, `pm-stretch`, `pm-vector` clean |

The 661 total includes the 21 passing pm-stretch tests (plus its inherited
ignored measurement), the 47 passing geometry/vector tests, and default
portable export tests. Counts from overlapping commands must not be added as
independent coverage. `color_ink` and several platform-gated export binaries
execute no Mac assertions on Linux; their zero-test exits do not qualify them.

The initial available replay had one actual coordinate-digest failure in
addition to the corpus blocker. The final six-test geometry replay and the
full 661-pass repeat supersede that failure without changing fixture bytes.
The preceding missing-CoreText compile failure was replaced with the explicit
failed target, not an ignored case or a substitute font.

Exact commands (Cargo commands from `engine/`; arrays only abbreviate the
repeated literal package flags):

```bash
engine_full_args=(-p pm-audio -p pm-cache -p pm-color -p pm-eval -p pm-export -p pm-expr -p pm-geom3d -p pm-glsl -p pm-gpu -p pm-media -p pm-protocol -p pm-proxy -p pm-reference -p pm-render -p pm-wasm)
engine_portable_args=(-p pm-cache -p pm-color -p pm-eval -p pm-expr -p pm-geom3d -p pm-glsl -p pm-protocol -p pm-reference -p pm-wasm -p pm-vector -p pm-stretch -p pm-export)
cargo test --locked --no-fail-fast "${engine_full_args[@]}"
cargo test --locked --no-fail-fast -p pm-node -p pm-vector
cargo test --locked -p pm-render --test stills additional_integer_decoder_classes_are_drawn -- --exact
cargo test --locked -p pm-audio --test playback cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous -- --exact
cargo clippy --locked --all-targets "${engine_full_args[@]}" -p pm-node -p pm-vector -p pm-stretch -- -D warnings
cargo test --locked --no-fail-fast "${engine_portable_args[@]}"
cargo test --locked --no-fail-fast -p pm-proxy --no-default-features -- --nocapture
cargo test --locked --no-fail-fast -p pm-export --no-default-features -- --nocapture
cargo clippy --locked --all-targets "${engine_portable_args[@]}" -- -D warnings
cargo clippy --locked --all-targets -p pm-proxy --no-default-features -- -D warnings
cargo fmt -p pm-eval -p pm-stretch -p pm-vector --check
mkdir -p ../work
rustc --edition=2024 --test crates/pm-audio/src/device_policy.rs -o ../work/device-policy-tests
../work/device-policy-tests
```

Additional commands from the repository root:

```bash
node --test engine/crates/pm-vector/tests/vectors/point-digest.test.mjs
python3 -m unittest discover -s engine/tools -p 'test_gate_e_inventory.py'
python3 -m unittest discover -s engine/crates/pm-audio/tools -p 'test_temp_root.py'
python3 engine/tools/gate_e_inventory.py
python3 engine/tools/gate_e_inventory.py --require-green
```

The inventory audit succeeds at 17 categories / 74 references. Its strict
`--require-green` command deliberately exits 1: **15 unqualified categories /
9 open work items**. Existing verification flags remain unchanged. The newer
latest-main/3D requirements are additional unqualified work, not automatic
coverage from this static inventory.

### Required Mac follow-up

Use the configured native SDK/FFmpeg/Python environment and same-checkout GLSL
helper from the original handoff. Run the full engine, node/vector and all-target
clippy commands above, plus the exact TIFF integration target. The new corpus
entrypoint's moved implementation must also be built/run on Mac.

For AUDIO-2, first measure with profiling/native diagnostics disabled. From
`engine/`, on the quiet Mac:

```bash
unset PM_AUDIO_PROFILE PM_STRETCH_DSP_DIAGNOSTICS PM_AUDIO_PLAYBACK_STRESS
cargo test --locked -p pm-audio --test playback cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous -- --exact --nocapture --test-threads=1
PM_AUDIO_PLAYBACK_STRESS=1 cargo test --locked -p pm-audio --test playback cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous -- --exact --nocapture --test-threads=1
```

Retain every raw miss against **10.666667 ms**, every complete 8,192-frame PCM
comparison, sixteen simultaneous voice starts, zero omitted prefix/A3 fires/
underruns, and the existing scheduling/restore/readback assertions. Repeat the
required plain/stress campaign and physical-device/A7/A8 rows. If a miss needs
attribution, collect a separate opt-in profile, then repeat qualification with
profiling off. Do not use the portable work probe as the deadline gate.

Original frozen evaluation/render/own-layer/text/reused-draw, font/layout,
mask/Canvas/SVG/WebGL and export/project witnesses remain necessary as detailed
above; latest-main adds its effect/version/resource and new-3D acceptance.
Native resource/sequence/host bindings, Chromium audio replacement and authored
export retiming, CHROMA-420-1, physical output/listening, 60/120 Hz and long-run
memory/cadence work remain open. These are not all explained by missing corpus.

### Portable native-work observation

Final quiet Linux probe (30 counterbalanced pairs, 16 independent stereo
histories per sequential cohort; all rendered PCM bits identical):

| Native work spans | p50 ms | p95 ms | max ms |
| --- | ---: | ---: | ---: |
| Original scalar output | 154.787 | 169.632 | 180.394 |
| Batched output | 152.026 | 154.102 | 166.410 |

These are sums of construction/seek/process spans on one thread across all
sixteen histories, **not parallel Play-to-first-buffer latency**. Input synthesis
and PCM collection are outside the spans. They show a work reduction in this
probe; they do not attribute the earlier Mac miss or qualify AUDIO-2.

```bash
# From engine/; optional diagnostic, no platform deadline assertion.
CARGO_TARGET_DIR=../work/stretch-target cargo run --locked -p pm-stretch --example cold_work -- 30
```


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
