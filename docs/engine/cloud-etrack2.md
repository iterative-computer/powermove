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
