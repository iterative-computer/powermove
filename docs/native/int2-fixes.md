# Integrated acceptance and work-counter repairs

Behaviour reference: `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`.
The tagged `v1.2.0` inventory remains separate and unchanged.

## Acceptance findings

The initial strict result was 1,655 passed, 28 failed and 2,661 blocked against
current main, with one additional failure in the legacy report. Every failed
assertion belongs to the following source-proven groups:

| Checks | Pinned source and finding | Repair or remaining boundary |
| --- | --- | --- |
| `MAIN.KEY.133`: definition and seven resolver contexts | `apps/desktop/src/extensions/keymap-default/keymap.ts:133`: backtick → `maximizePanel`, priority 100; the native action was missing. | Maximize the panel under the pointer using transient geometry, preserving the saved docking layout. |
| `MAIN.KEY.198`: definition and seven resolver contexts | `apps/desktop/src/extensions/timeline/index.ts:148–155`: M → `toggleLayerStrips`, priority 90; its body calls property reveal. `property-reveal.ts` defines selected masks, double presses and global disclosure. | Route through the built-in timeline model with monotonic press timestamps. Persist disclosure without writing animation or document history. |
| `MAIN.KEY.196`: definition and seven resolver contexts | `apps/desktop/src/extensions/viewer/index.ts:44–57` and `viewer-chrome.ts:59–102`: Tab opens the composition mini-flowchart and opens the chosen composition. | Still fails. Native lacks the full composition navigation host, including saved per-composition view state and history-preserving switching. Merely registering the name would conceal missing behaviour. |
| `MAIN.curve.set`, the strict main easing declaration, and legacy `W061.frozen-set-easing` | `apps/desktop/src/renderer/src/legacy/core/editing.ts:272–314,1134–1150`: collect named presets through each uninterrupted property run, then finish against final neighbours before other operations read or override animation. | Fix native batch finishing. The unchanged independent f64 curve oracle now passes; midpoint rotation matches the independently expected 104.94929544957891 within its existing bound. The complete agent-run declaration returns to its existing controller blocker. |
| `MAIN.MCP.edit_3d.catalog`, `MAIN.MCP.tools.identity` | The frozen main catalog describes the main `edit_3d` surface; the integrated Studio overlay adds operations and schema fields. | Exact pinned identity remains visibly different. Studio wiring and its catalog reconciliation belong to the separate hooks lane; no catalog pin was weakened. |

The seven resolver contexts are normal, held, extra modifier, input,
contenteditable, prior decline and key up. All original 195 legacy bindings and
all original case IDs remain. Two production bindings are additive. Native
window/keyboard integration remains unqualified by this headless work.

Measured supporting acceptance: **1,672 passed / 10 failed / 2,662 blocked**.
The legacy supporting report has **1,424 passed / 0 failed / 3,179 blocked**;
the configured runtime result is recorded below.

## Remove unnecessary work

- RGB sources bind their existing view in the unused chroma slot. Packed RGBA
  stills perform decode-in's exact code-value normalization inside the source
  conversion pass, avoiding an intermediate texture, dispatch and uniform
  upload. Planar stills return normalization scratch after recording its reads;
  subsequent commands reuse it in queue order. Native-depth, alpha, transfer,
  primaries, HDR and mip-generation semantics remain active.
- Single-item shape scenes avoid a second geometry-key preparation walk that
  cannot supply enough independent outline misses to parallelize.
- The 3D composite shader excludes unused analytic-solid functions. Half-float
  solid/rectangle shaders exclude unrelated color-transfer functions. Required
  nearest-even half conversion remains intact.
- Independent font registries share copied font-file snapshots. The cache is
  bounded to 16 MiB and 64 files, validates path/device/inode/size/modification
  and change times, and retains immutable old bytes for existing users after a
  replacement. Sealed system-font mappings retain their existing path.
- Guarded audio preparation reuses exact FFT coefficients per worker and call
  site. Each retained table is bounded to 32,768 coefficients. Reuse has no
  process-wide mutex, preserves cooperative checkpoints, and changes no DSP
  arithmetic, FFT backend, seed, pre-roll or callback deadline. Ordinary setup
  remains the independent bitwise comparison path.

All 17 performance benches executed in the initial reproduction. It reproduced
22 previously reported bounds and two additional drag heap-byte bounds. A
subsequent intermediate run reduced failures to 17, then the remaining
optimizations reduced them to eight. The required-feature baseline changes
below cover those eight fields; no tolerance changed. Temporary allocator
instrumentation was removed.

## Qualification boundaries

The audio deadline remains **10.666667 ms**. The first coefficient-cache run
measured 10.734, 9.688, 10.423 and 9.631 ms; the first trial still fails.
A later passing suite does not erase that miss. Timing needs a quiet-machine
run of all four unchanged trials and the broader playback suite. Complete PCM,
simultaneous starts, omitted-prefix/spread/underrun and scheduling checks remain
mandatory.

Missing evaluator/vector/SVG/captured-font references, no-input corpus
qualifications, and the prohibited window-capture checks retain their existing
blocker status. No references were substituted, recaptured or manufactured.

The merge gate is not met while the ten acceptance failures remain. Fixes must
stay on `feat/na-mac-fixes`; no release or main-branch action is part of this work.

## Required-feature baseline changes

After removing redundant passes, uploads, reads and shader functions, eight
bounds still represent required rendering work. Only these fields are raised;
all bench units, cases, other bounds and the existing tolerance table stay exact.
The first complete cold-bench baseline (`a3938c007`) predates the nearest-even
half-store work in `93d42e636` and the native-depth still integration.

| Bench / counter, both scopes | Previous | New | Required feature and evidence |
| --- | ---: | ---: | --- |
| `export_120 gpu.texture_alloc_bytes` | 354,073,088 | 354,946,896 | Native working-precision still textures and their physical mip chains replace the old `rgba8_bitmap` compatibility path (`05d9e38e9:pm-render/src/media.rs:1243–1251`). The source remains native-depth; fused normalization restores the original 28 texture calls and 3,122 uploads. The measured remaining difference is 873,808 bytes across the full export. |
| `blend_stack_cold cpu.heap_alloc_bytes` | 1,419,648 | 1,551,613 | Compile the required nearest-even half-store shader. Solid/rectangle variants exclude unused color functions and the 3D module excludes all unused analytic-solid functions; the conversion itself remains required for correct half-float values. Allocator traces identify first-use WGSL parsing in `HalfModules::module`. |
| `text_cold cpu.heap_allocs` | 10,800 | 11,744 | First-use half-store shaders and exact outline preparation. Traces identify the half module's WGSL parsing and glyph geometry. The duplicated 6.26 MB font read is removed by the bounded snapshot cache. |
| `text_cold cpu.heap_alloc_bytes` | 1,392,503 | 2,575,841 | Same required cold text work. This is the measured all-thread allocation total; either the caller or helpers can execute a glyph, so both scopes use that measured total rather than relying on which worker claimed it. The measured caller-only total was 2,142,841 bytes. |

Import-3D heap bytes, video-pyramid heap bytes, shape allocations, all open-large
bounds, export allocation counts and export uploads meet their original bounds;
none of those baselines are raised. The parallel text heuristic tested during
investigation was reverted when it did not materially improve the whole counter.

## Final headless verification

The complete performance ratchet passes: **17 benches / 0 failing bounds** with
only the eight required-feature baseline fields above changed. Every benchmark
ran; no case filter or update mode was used.

Affected results: app/builtins **402 passed / 0 failed / 1 existing ignored**;
shell **36 passed**; timeline v4 **19 passed**; renderer **311 passed**, plus
successful `stills2` and `video_reading` standalone targets; audio/stretch
**267 passed / 0 failed / 2 existing ignored**; text library **37 passed**.
Renderer checks include byte-identical packed-still fusion versus the separate
normalization path across all 256 code values, four source storage formats,
both working spaces and all three output precisions. These counts do not
qualify window input or the audio timing deadline.

Strict verification completed with **1,672 passed / 10 failed / 2,662 blocked**
against the pinned main reference and **1,467 passed / 0 failed / 3,136 blocked**
for configured legacy compatibility. Strict exit 1 preserves the ten failures.
All **139 Rust tests and 8 Python tests** in its prerequisites pass; formatting
and crate-local Clippy pass. Affected engine all-target Clippy also passes.
The final animation-command suite has **27 passes**, and the timeline rerun has
**19 passes**. No visual, corpus or quiet-machine blocker is promoted.
