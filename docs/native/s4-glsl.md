# S4: GLSL compiler isolation and inventory

Status: **available-corpus draw regression passes; S4 is not complete**.
The native Metal compiler still runs in the production device-owning process.
The historical phase-3a pixel-oracle capture is unavailable on this machine.
Both items must be resolved before the complete P0 S4 gate can pass.

## Decisions and their rules

| Decision | Rule that selects it |
|---|---|
| Preserve shader text, coordinate transforms, logical resolution, `u_res`, reflection, uniform calls and Metal math options. | N3 and SHADER-RES-1. This spike changes compiler execution, not rendering geometry or sampling. |
| Keep the existing WebGL fallback for valid programs whose resources the native compositor cannot bind. | N4 and S4: WebGL is the final route before a placeholder. A bind refusal is not evidence that the shader itself is invalid. |
| Do not add Boolean/float coercion to accept malformed GLSL. | N3 and §4: the language specification wins; an invented conversion changes authored formulas. |
| Use two killable process slots with immediate admission failure and no waiting queue. | S4: timed-out glslang/naga work must not accumulate. Each slot starts a fresh single-request worker, avoiding compiler state leaking between programs. |
| Move cached finishing and the downstream naga MSL writer into that same pool. | S4 explicitly includes `Prepared::finish()` and downstream compilation in the compiler boundary. |
| Do not prototype SPIRV-Cross. | S4 permits it only for naga failures remaining after targeted fixes. There are **zero naga parse/validation/writer failures** in the available inventory. |
| Leave the complete gate open instead of substituting a native preflight for production isolation. | N3/N4 and S4: successfully compiling a copy in a helper does not bound a second compilation in the host. |

No pm-gpu changes. The only production renderer change redirects its existing
MSL writer to pm-glsl's process pool. Shader formulas and SPIR-V emission are
unchanged; the existing pipeline-revision fingerprint still passes.

## Inventory

**661 case occurrences, 611 distinct wrapped sources, 1,605 variants.** Occurrences
are counted separately across the shader, effect and coverage corpora; the
generated effect corpus overlaps the shader corpus. Columns count programs
successfully completing that stage. Every successful program's collected
variants and passes completed Metal draw and raw readback.

| Input group | Programs | Variants | Validation | glslang | Rewrite/bind | naga | Metal | Draw |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Built-in effects | 16 | 176 | 16 | 16 | 16 | 16 | 16 | 16 |
| Built-in transitions | 4 | 52 | 4 | 4 | 4 | 4 | 4 | 4 |
| Built-in fragment layers/placeholder | 2 | 6 | 2 | 2 | 2 | 2 | 2 | 2 |
| Shader presets | 5 | 15 | 5 | 5 | 5 | 5 | 5 | 5 |
| Docs samples | 3 | 11 | 3 | 3 | 3 | 3 | 3 | 3 |
| Cloud samples | 5 | 20 | 5 | 5 | 5 | 5 | 5 | 5 |
| e2e literals | 19 | 68 | 17 | 17 | 16 | 16 | 16 | 16 |
| Unit-test literals | 23 | 85 | 22 | 22 | 22 | 22 | 22 | 22 |
| Scene shaders | 2 | 8 | 2 | 2 | 2 | 2 | 2 | 2 |
| Generated S3 | 56 | 212 | 54 | 54 | 54 | 54 | 54 | 54 |
| Generated agent | 35 | 112 | 33 | 33 | 32 | 32 | 32 | 32 |
| Generated effect/transition sweep | 43 | 219 | 41 | 41 | 41 | 41 | 41 | 41 |
| Explicit fallback fixtures | 3 | 23 | 3 | 3 | 0 | 0 | 0 | 0 |
| GLSL coverage probes | 394 | 394 | 291 | 291 | 265 | 265 | 264 | 264 |
| Public Store releases | 5 | 22 | 5 | 5 | 5 | 5 | 5 | 5 |
| Installed extensions, all entry formats | 44 | 180 | 43 | 43 | 43 | 43 | 43 | 43 |
| Generated GLSL rejected earlier by registration schema | 2 | 2 | 2 | 2 | 2 | 2 | 2 | 2 |
| **Total** | **661** | **1,605** | **548** | **548** | **517** | **517** | **516** | **516** |

Weighted by variants: validation/glslang **1,466**, rewrite/naga **1,410**,
Metal/draw **1,409**. Source limits are included in validation. The rewrite
column also includes resolved SPIR-V limits and the resource-binding check.

Collection uses the existing repository collectors and their read-only
activation sandbox. Installed sources were never modified. The entry audit
covered 46 directories: 43 TypeScript entries, one manifest-directed JavaScript
entry and two empty directories. The supplemental collector adds that JavaScript
entry and two generated fragment programs that the original collector discarded
for parameter-schema errors. Those three supplemental programs use one smoke
variant each, not captured interaction traces. Four activation-limited
TypeScript entries were inspected; they contain no GLSL definitions. One partial
e2e definition with an unevaluable spread remains excluded by the original
collector; this inventory does not claim exhaustive dynamic registration tracing.

Four current public Store releases were fetched without credentials. Their five
programs have 22 variants. The historical [phase-3a shader gate](../engine/phase3a-gate.md)
recorded **144 sources / 522 variants: 504 matching, 3 through fallback, 15 refused
by both, 3 WebGL-unstable** (fallback overlaps matching). Its saved capture and
reference pixels are absent here. Recollection covers the available sources at
this tip; it does **not** reproduce that exact historical source set or certify
its pixel oracle.

## Failure classification and targeted fixes

| First failed operation | Programs | Classification/action |
|---|---:|---|
| Source limits | 5 | Existing resource/language limits; diagnostics retained. |
| GLSL ES validation | 108 | Syntax/type/link failures. Includes one installed program with an invalid Boolean/float comparison; no shader rewrite to hide it. |
| glslang Vulkan compilation | 0 | No additional normalization fix needed. |
| SPIR-V rewrite itself | 0 | No additional rewrite needed. |
| Resource binding, before rewrite completion | 31 | Unsupported sampler/resource contracts. Eight coverage probes are recorded as drawable by WebGL; 18 have recorded WebGL draw errors. The other five are explicit fallback/example cases. |
| naga parsing/validation/MSL writing | 0 | No residual translation problem warranting SPIRV-Cross. |
| Native Metal pipeline compilation | 1 | Integer fragment output against RGBA16F. Correct refusal, also a recorded WebGL draw error. The existing renderer reports this using its older `naga-validate` label; the inventory attributes it to Metal after successful naga completion. |
| Draw/readback | 0 | Every successfully compiled pipeline completed its variants. |

**13 case occurrences / 38 variants** still require the existing WebGL route:
eight otherwise-drawable coverage probes plus five fallback/example cases. The
18 other bind probes and integer-output probe already have recorded WebGL draw
errors. The new native compat page's fallback is not implemented or tested here.
No placeholder substitution was added.

The common failures warranting code changes were execution-boundary failures:

- Replace abandoned translation threads with kill-and-reap worker ownership.
- Route public prepare/translate, cache misses, cached `Prepared::finish()` and
  production naga-to-MSL writing through the pool.
- Do not retain transient busy/process/resource/deadline failures in the cache.
- Reject oversized disk-cache entries before reading them into memory.
- Keep the existing fuzz binary's compilation inside its own supervised worker,
  avoiding a nested compiler child that could outlive a killed fuzz worker.
- Add named `compiler-busy`, `compiler-process` and `compiler-resources` failures;
  preserve existing source-line diagnostics and `deadline` failures.

## Compiler process boundary and verification

`pm-glsl-compiler --glsl-compiler` handles one request and exits. The host owns
an inherited Unix socket, not temporary source files or a listening endpoint.
A length-prefixed protocol caps requests and responses at 64 MiB. Workers do
not inherit credentials or the caller's environment. No in-process compilation
fallback runs when the helper is absent.

The default host pool admits at most **two processes**, with no queued work.
Its **20-second deadline applies per request**, including cached finishing and
MSL writing. Timed-out, crashed, malformed-response and over-budget workers
are killed and reaped before their slot is released. glslang's large stack
remains inside the worker. CPU time is limited to 30 seconds and core dumps
are disabled. Darwin rejected finite address-space/data hard limits in the
spike; the parent instead samples resident memory during IPC at intervals up
to 50 ms and kills a worker above 1 GiB. Sampling permits transient overshoot.
A monitoring failure fails closed. A host may configure a lower resident budget.

Ship the helper beside the host executable, or set the trusted host setting
`POWERMOVE_GLSL_COMPILER` to its absolute path. Cargo `deps`/`examples` layouts
are recognized for development. App packaging is outside this spike's edits.

Verification, Rust **1.96.1**, shared configured target directory:

- Before changes: all **45** existing pm-glsl unit tests and both integration
  tests passed: the **394-probe** verdict sweep and **300** deterministic mutants.
- After changes: all **45 + 5 new process tests + 2 existing integration tests**
  pass, including ordinary parallel `cargo test -p pm-glsl` execution.
- Six relevant existing pm-render `glsl::` tests pass with the original writer
  and with the isolated writer.
- Admission stress occupies both slots, rejects excess work immediately, and
  repeats six real worker timeouts over three rounds; admission returns to zero.
- Cached SPIR-V finishing can be timed out. Its successful MSL exactly matches
  the original writer on a uniform/coordinate fixture.
- A low resident-budget worker allocating memory is killed with
  `compiler-resources`; later valid requests succeed.
- Missing/exited helpers fail by name. Adversarial sources include the existing
  macro/array/recursion bombs, a huge loop candidate and 100,000 nested
  parentheses. Under short deadlines they fail safely or return a compiler
  verdict; the next valid request still succeeds. Short deadlines exercise
  cancellation without assuming a compiler will actually unroll the loop.
- Comparing **all 661 cases** with the original production MSL writer gives
  **zero compile/draw verdict changes**, including every collected variant.
  Pixel digests agree for **515 of 516** native-drawn cases. One private case
  also changes pixels between repeated runs of the *same* writer; its original
  and isolated MSL were verified byte-identical. This is recorded as one native
  self-unstable case, not a demonstrated shader-correctness pass.
- No compiler workers remained after the completed test/inventory runs.

The inventory harness encloses native Metal compilation, draw and readback in
a disposable process with a 30-second case deadline. It binds the production
`GpuProgram` layouts/reflection, replays collected uniform calls and passes,
uses corpus images, and checks GPU completion. It uses a fixed linear clamp
sampler for the inventory; this is a compilation/draw gate, not the full
production sampling/visual oracle.

### Remaining production boundary

Production `GpuProgram::pipeline()` still calls wgpu's native Metal pipeline
creation in the device owner. Moving the naga MSL writer out does not isolate
that call. wgpu-hal 30's Metal pipeline cache is a stub, and its public API does
not import a pipeline compiled in another process. Returning naga IR or MSL
would compile it again in the host. A helper preflight therefore cannot close
this requirement.

Closing S4 needs a reviewed backend change that consumes precompiled native
artifacts without silently invoking the compiler in the host, or a separately
owned GPU execution service. That backend/device-lifecycle work exceeds the
small pm-render integration authorized here. Until it exists, native compiler
hangs and adversarial Metal unrolling are **not proven contained** in production.

## SPIRV-Cross safety contract, if later required

No prototype or dependency was added. A future route is eligible only after ES
validation, glslang and resource reflection succeed, and naga specifically
fails. It must retain the original pre-naga SPIR-V and its reflection rather
than applying naga-only rewrites to an independent compiler's input.

Required before enabling it:

1. Use the same process admission, IPC/source limits, CPU/RSS budgets and hard
   wall timeout through SPIRV-Cross and the native compiler. Name translation,
   reflection, resource, native-compile and deadline failures separately.
2. Reflect every descriptor set, binding, sampler, block offset, matrix stride
   and array stride from SPIR-V. Bind against the actual pipeline layout;
   reject an unsupported resource instead of inventing a binding.
3. Enable and verify robust buffer/vector/array/image indexing, initialization
   and integer-division checks. Test negative/large indices, unsigned wrap and
   zero division. Passthrough MSL carries no implicit wgpu safety validation.
4. Bound loops without changing normally terminating authored programs. An
   iteration ceiling alone is not a wall-time guarantee. Fence draws and poll
   completion under a deadline before readback. On timeout/device loss,
   retire the device and its surfaces, recreate it, quarantine the unchanged
   failing source and notify callers; jobs fail by name rather than omit pixels.
   Killing a CPU helper does not itself cancel an already submitted GPU kernel.
5. Headless draws must cover coordinates/`u_res`, derivatives/origin, sampler
   arrays, reflection/bounds cases, ordinary shaders, huge-unroll candidates
   and non-terminating shader loops, including device-loss recovery and a
   subsequent good draw. Preserve WebGL as the final compatibility route.

## Gate verdict and reproduction

**Available-corpus “no previously accepted program stops drawing”: PASS.**
**Complete P0 S4: OPEN**, because production native Metal compilation is not
killable, historical WebGL reference pixels are unavailable, and the private
self-unstable case does not have a shader-correctness verdict. Registration
tracing and actual compat-page fallback behavior also remain outside this probe.
The orchestrator's review and in-person gate verification remain required.

From `engine/`, after sourcing `engine/env.sh` from the repository root and
setting `CARGO_TARGET_DIR=~/Developer/pm-target`:

```sh
cargo build -p pm-glsl --bin pm-glsl-compiler --example inventory
cargo test -p pm-glsl
cargo test -p pm-render --lib glsl:: -- --test-threads=1
```

Collect outside Git using the existing `pm-render/tools/effects/collect.ts` and
`pm-render/tools/shaders/collect.ts` (with public Store capture and optional
read-only local extensions). `pm-glsl/examples/supplement.ts` supplements entry
formats and schema-rejected generated fragments. Run the inventory executable
with one or more corpus directories. `PM_GLSL_INVENTORY_PRIVATE_RESULTS` writes
optional private per-case results outside the repository; stdout is aggregate
counts only. `PM_GLSL_VERIFY_MSL` compares original/isolated writer output;
`PM_GLSL_INVENTORY_PIXELS` is an optional private raw-pixel directory for a
single `--case <directory> <index>` investigation. On this development setup,
FFmpeg-dependent test/example binaries also need the configured FFmpeg library
directory in `DYLD_LIBRARY_PATH`.
