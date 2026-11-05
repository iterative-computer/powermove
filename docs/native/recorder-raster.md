# P2 package 11, part 2: CPU recorder raster execution

`apps/native/crates/recorder-raster` (`pm-recorder-raster`) builds on
`feat/na-cloud-recorder`. It executes immutable `pm-recorder::Command` streams
on a worker, exposes version-checked replay and synchronous pixel readback, and
adds a float Gaussian shadow primitive. No Node, Chromium, V8, WebKit, Metal,
AppKit or GPUI dependency is added to the app. Chromium is an optional fixture
generation tool only. The kernel remains the document's single writer.

This is a reusable CPU execution package, **not v1–3 cutover certification**.
In particular the existing recorder does not yet capture shadow state or provide
pre-clip source alpha. The shadow primitive is executable and tested here;
automatic shadows on existing recorded draw calls require the upstream API
changes described below. No recorder, engine, shared workspace/lockfile,
application, vendor or spike source is changed by this lane.

## Boundary

`Rasterizer::new(id, width, height)` admits the recorder's sparse backing and
67,108,864-pixel limit, including 8192×8192. Canvas ids are host-assigned and
must never be reused for different canvases. `key()` is the inspectable
`SourceKey { id, version }`; `size()` reports admitted dimensions.

`replay(expected_key, commands)` accepts only the unconsumed suffix at that
exact identity/version. It rejects stale versions before changing pixels;
the version advances by the command count. Commands and paints retain immutable
source versions through `Arc` leases. Retiring or dropping the producer never
invalidates an outstanding batch. Empty replay preserves the version. This is
an in-process typed Rust stream, not a new serialized/wire format.

`image_data()` and signed `get_image_data(sx, sy, sw, sh)` return independent,
straight-alpha sRGB RGBA8 buffers. Replay keeps float premultiplied storage
between draws, batches and reads; it does not round-trip through ImageData.
Out-of-bounds pixels are transparent black. Reads ignore all drawing state.
Zero read extents return `IndexSize`; zero-dimensional canvases remain valid
and return transparent nonempty regional reads. `Command::Put` preserves the
recorder's state-independent dirty putImageData behavior. Resize, including
same-size resize, clears pixels; failed admission retains the old bitmap/key.

The raster backend directly reuses `pm-recorder::CpuRasterizer`. It already
uses pm-vector for fill-rule resolution, transformed stroke expansion, joins,
caps, dashes and geometric clip intersection, and pm-text for font resolution,
HarfRust shaping and outline extraction. Analytic polygon coverage integrates
flattened geometry at the existing 1e-4 device-pixel tolerance. Linear/radial/
conic gradients, pattern repetition/transforms, cropped nearest/bilinear images,
globalAlpha, all 26 composite modes, clear and put are replayed through that
same backend. `recorder::TextProvider` is the injected font boundary;
`recorder::TextEngine<R>` accepts a native font resolver or Linux test double.
Shaping occurs during recording, so replay does not call back into a font host.

## Shadow primitive

`AlphaMask::new(width, height, origin, values)` takes finite f32 alpha in
[0,1] in device space **before the final canvas clip**. Source paint alpha,
image alpha, geometry coverage and globalAlpha must already be included,
exactly once. Keep off-canvas casters and padding: a caster outside the bitmap
can cast a shadow into it. Inputs are copied/owned and never mutated by blur.

`blurred(shadow_blur)` uses the HTML Canvas sigma = blur/2, separable normalized
Gaussian convolution with f64 accumulation and f32 storage. Five-sigma support
omits less than 5.8e-7 of the continuous one-dimensional mass; the normalized
discrete kernel is the specified numerical model here. Fractional offsets use
bilinear alpha reconstruction after convolution. There is no 8-bit intermediate
mask, downsampling, or CTM scaling of the shadow offset/blur. The widened plane
is admitted against the pixel limit and an explicit 100,000,000-tap work limit;
excess returns `Capacity`, never a silently smaller blur. Extremely small blur
and integer-origin overflow are handled explicitly.

`draw_shadow(expected_key, mask, shadow, composite, device_space_clips)` paints
only the shadow, then the caller replays the original source draw. It advances
one logical effect version. Shadow color alpha multiplies source alpha once.
Final clips intersect **after** blur/offset. The float plane is composited as
integer-aligned constant-alpha spans through the retained float backend; each
span has its own geometric clip. This preserves fractional clip coverage and
allows copy/source-in/source-out/destination-in/destination-atop to process
transparent source exactly once throughout the clip. No ImageData quantization
is used to feed the shadow back into the renderer. A zero-size recorder serves
only as a bounded command builder; consumed spans are compacted immediately.

This explicit primitive always composites the supplied mask, including zero
offset/blur. The Canvas binding must decide whether a shadow is enabled from
shadowColor/offset/blur according to HTML, skip shadows for clear/put, capture
save/restore/reset behavior, and handle the source/shadow drawing order. Do not
count the extra effect version as a recorder command version without an adapter.
This package does not silently infer shadow state from a private DrawCommand.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Reuse the existing analytic backend instead of adding another tiny-skia renderer | N1/N2 quality; N3 one geometry/paint implementation; prune rule retains current consumers |
| Keep Canvas compositing in float premultiplied sRGB code values, distinct from the engine's linear HDR space | N3 authored Canvas colors; N6 Canvas contract |
| Version-check worker replay, retain immutable source leases, preserve state on rejection | N3 exact input identity; N4 no lost work; N7 drive/inspect by id |
| Keep synchronous execution/readback independent of a UI or JS engine | N6 sync contract; N7 headless; N8 main thread never waits on JS |
| Implement shadows from pre-clip float alpha, including off-canvas casters | N1 pro quality; N3 Canvas drawing model rather than cropping artifacts |
| Prefer exact area/filter arithmetic over reproducing Chromium edge and filter-phase approximations | N3 and quirk policy: no recorded intent requires these artifacts; N2 better numerical quality |
| Add independent numeric fixtures alongside bounded Chromium visual comparisons | N1/N3 evidence distinguishes geometry/color intent from browser approximation |
| Expose missing recorder/host APIs and capacity failures rather than inventing JS errors or dropping draws | N3/N4/N6 valid extension calls still require a synchronous supported route |
| Use an isolated, engine-aligned lockfile within the owned crate | Lane ownership; shared native lockfile regeneration belongs to integration |

## Needed changes outside this lane

1. **Recorder shadow capture.** Add shadowColor, shadowBlur and offsets to saved
   drawing state and frozen draw commands. Expose a native pre-clip float source
   alpha render hook (with bounds/origin, off-canvas casters and globalAlpha),
   plus immutable device-space clip snapshots. Then invoke this primitive before
   the source draw on the same worker. Text/image/path sources need the same hook.
   The native-only primitive is not permission to expose an invented exception
   or omit a valid shadow. Until this is wired, retain an appropriate synchronous
   native fallback; this lane does not supply or certify that runtime fallback.
2. **Regional readback.** Expose the existing private
   `CpuRasterizer::read_into(x, y, &mut ImageData)` as a public safe method.
   This package currently has to convert the full frame for an overlapping
   regional read. Full-frame requests avoid an extra copy; entirely out-of-bounds
   requests avoid backend conversion. A one-pixel in-bounds read still costs a
   dense full-frame conversion. SIMD/direct-tile conversion is a performance
   follow-up, not a changed pixel contract.
3. **Workspace integration.** Regenerate `apps/native/Cargo.lock` to admit this
   crate, pm-recorder and its engine dependencies. The existing `crates/*` glob
   already discovers the production manifest. The requested workspace command
   with `--locked` refuses to change that unowned lockfile. The isolated harness
   points at the identical production sources and tests; shared pins start from
   the recorder's engine-aligned lockfile.
4. **Host integration.** Bind immutable source ids/versions, nested V8 raster
   continuations, font readiness and sandbox-owned buffers; connect engine-service
   upload/presentation without putting JS on the main thread. Existing recorder
   gates for CSS style-source/wide-gamut colors, color glyphs/variations, filter,
   alpha=false/P3/float16 contexts, decoding/security/taint, bitmap lifecycles,
   promotion and serialization remain gates. No existing consumers are pruned.

References: [recorder](recorder.md), [architecture](p0-decisions.md),
[C-MATRIX](c-matrix.md), [S1](s1-v8.md), [S5](s5-compat.md),
[engine service](engine-service.md), [HTML Canvas](https://html.spec.whatwg.org/multipage/canvas.html),
[Compositing and Blending](https://www.w3.org/TR/compositing-1/).

## Visual and numeric oracles

The checked-in fixtures contain only synthetic geometry, colors, 2×2 image
pixels, and the recorder's generated geometric font. `tests/fixtures/generate.py`
uses an isolated headless Chromium profile through loopback CDP; it never reads
an existing browser profile, project or installed font. The browser version
and SHA-256 of each of the 13 PNGs are in `manifest.json`. Text uses the same
synthetic TTF as the Linux `FontResolver` double. Reference pixels come directly
from Canvas `toDataURL`, not a screenshot or page scaling.

`tests/fixtures/analytic.py` independently generates seven PNGs and an
`analytic-manifest.json`. It uses elementary horizontal circle/ellipse
intersections, inverted quadratic/cubic equations, separable rectangle area,
explicit bilinear filter taps and a direct two-dimensional Gaussian with wider
support than the implementation. Area integration uses 16,384 midpoint
ordinates per pixel. Expected unorm8 values were checked against the described
geometries/equations; this generator never calls recorder/engine geometry or
Chromium. The strict tests allow at most **1/255** per premultiplied RGBA channel;
all seven currently match **byte-for-byte**, including the Gaussian shadow.

Chromium comparisons are a separate visual smoke oracle. Deltas below are
in 0–255 premultiplied RGBA units. Transparent straight RGB is not meaningful;
comparison premultiplies before measuring error. All images additionally require
mean absolute channel error ≤1.5. Fractional edge tolerance applies only where
alpha is fractional in either image **and** a neighboring pixel has different
alpha. Constant translucent interiors keep the 1-unit tolerance.

| Reference | Per-channel tolerance | Observed max / mean |
|---|---:|---:|
| Rectangles, globalAlpha and clear | 1 interior; 80 at fractional edges | 1 / 0.0745 |
| Linear / radial / conic gradients | 2 | 1 / 0.1250; 1 / 0.1139; 1 / 0.1243 |
| All pattern repetitions | 1 interior; 80 at fractional edges | 0 / 0 |
| All 26 composites, regular draws and float shadow spans | 3 | 2 / 0.1346 |
| Nearest images / dirty put | 1 | 0 within that region |
| Enlarged bilinear image, only its explicit destination region | 20 | 18 / 0.6615 for the whole fixture |
| Caps, joins, miter limit, odd dashes/offset | 1 interior; 32 edge | 25 / 0.2480 |
| Arcs, ellipse and arcTo | 1 interior; 64 edge | 59 / 0.6016 |
| Signed roundRect | 1 interior; 48 edge | 39 / 0.3037 |
| Curves/fill rules | 1 interior; 80 edge | 80 / 0.7155 |
| Transformed intersecting clips | 1 interior; 80 edge | 78 / 1.3708 |
| Shaped fillText/strokeText, baseline, alignment, maxWidth | 1 interior; 16 edge | 12 / 0.0794 |

These are intentionally **not** pixel-equivalence claims for Chromium curves,
clipping or smooth images. Chromium's curved-edge approximations and filter
phase quantization differ visibly at individual edge pixels. The independent
strict area/image fixtures, not a widened browser tolerance, select the numeric
implementation under N3. No saved intent or product decision in the supplied
plan requires reproducing those artifacts. A future authored-intent exception
must name its own fixture/decision rather than broadening these tolerances.

Regenerate with Python 3, the `websockets` package and headless Chromium:

```sh
python3 apps/native/crates/recorder-raster/tests/fixtures/generate.py --chromium /usr/lib/chromium/chromium
python3 apps/native/crates/recorder-raster/tests/fixtures/analytic.py
```

PNG regeneration is explicit; tests only read the checked-in files. Hashes make
regeneration reviewable. Both generators reproduce their PNGs byte-for-byte on
this runner. No user/profile data or browser binary is checked in.

## Linux validation

Pinned Rust **1.96.1**, Linux x86_64:

- **128 tests pass**: 15 new raster/shadow integration tests, 21 new PNG tests
  (13 Chromium scenes, seven independent analytic images, and a second use of
  the composite fixture for all 26 shadow operations), plus all 92 unchanged
  recorder contract tests. No ignored tests.
- All-target package Clippy with `--no-deps -- -D warnings` and formatting pass.
- The requested `cd apps/native && cargo test --locked -p pm-recorder-raster`
  was attempted and refused shared lockfile regeneration. This is not reported
  as a workspace pass. The isolated manifest builds the same library/tests with
  a checked-in lockfile and no app/platform dependencies.
- No engine code changed. The recorder documentation's existing pm-vector
  platform digest failure is still an integration follow-up; this lane does
  not claim to have repaired it or certified the entire engine suite.

From the repository root, with Rust 1.96.1 on PATH:

```sh
cargo test --locked --manifest-path apps/native/crates/recorder-raster/isolated/Cargo.toml
cargo clippy --locked --manifest-path apps/native/crates/recorder-raster/isolated/Cargo.toml --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/recorder-raster/isolated/Cargo.toml --check
cargo run --release --locked --manifest-path apps/native/crates/recorder-raster/isolated/Cargo.toml --example ops
```

## Native microbenchmark

`examples/ops.rs` uses `Instant`, 20 warmups and 101 timed samples per operation,
with median/p95 and a retained float destination; it does not include recording,
shaping, JS, GPU upload or presentation. A named scene is one replay operation,
not one individual Canvas call. Counts: rects 5 commands; curves 3; arcs 4;
roundRect 2; strokes 7; clip-transform 2; each gradient 1; patterns 4; images 3;
composites 52; text 4. Input recording/shaping is outside timing. The shadow
row includes Gaussian generation and native compositing of all its spans.

Full machine-readable results are in
`apps/native/crates/recorder-raster/benches/linux-x86_64.csv`. Selected release
measurements on this shared Linux x86_64 container, **ns/op**:

| Operation | Median | p95 |
|---|---:|---:|
| One fillRect, 16×16 | 3,594 | 3,708 |
| One linear gradient, 32×24 | 25,743 | 25,846 |
| One radial gradient, 32×24 | 34,476 | 35,089 |
| One conic gradient, 32×24 | 41,609 | 56,072 |
| Seven stroke commands, 32×24 | 78,958 | 100,883 |
| Four shaped text commands, 32×24 | 12,217 | 12,909 |
| Dirty putImageData, 16×16 | 1,851 | 1,969 |
| Exact drawImage copy, 1920×1080 | 4,692 | 5,484 |
| Full RGBA output from immutable copy base, 1920×1080 | 876,786 | 1,910,400 |
| Full RGBA output from dense float tiles, 1920×1080 | 18,236,366 | 32,450,435 |
| One-pixel read from dense 1080p | 18,994,902 | 21,494,494 |
| Gaussian mask, 16×16, blur=4 | 19,836 | 24,267 |
| Shadow, 16×16 caster, blur=4, 48×48 output | 2,539,573 | 3,651,826 |

Dense readback and the span-based shadow path need optimization before broad
production use. These results do **not** establish a speedup over Electron or
pass EXT-RASTER-1. JS/whole production path are unmeasured, and the dense-readback
row exceeds the 4-ms whole-path target by itself on this Linux runner. The exact
copy fast path is inherited, not a new approximation. No image cache is reused
under a different source version. The main-thread/worker architecture is not a
license to hide this performance gate.

## macOS verification still required

Verify native font matching/fallback/variations and project font lifecycle;
actual V8 sync Canvas bindings and sandbox-owned pixel buffers; nested raster
continuations; held canvas/context/paint identity across real remote-DOM and
WebKit promotion; decode origin/taint and bitmap lifecycle; shadow-state capture
and save/restore/reset; engine-service upload/color conversion and Metal
presentation. Exercise the extension corpus, including the shipped timeline's
blurred shadows, on supported Macs. Measure JS recording ≤1 ms p95 per changed
layer and the complete EXT-RASTER-1 frame ≤4 ms p95. Linux has no AppKit, GPUI,
WebKit, Metal or display proof and cannot close these gates.

## 2026-11-04 — runtime2 direct regional conversion

The recorder now exposes `CpuRasterizer::read_into`, and signed regional reads
call it directly. This closes item 2 in the older "Needed changes outside this
lane" list above. An in-bounds one-pixel read no longer requires a full-frame
RGBA allocation or conversion. Existing signed-extent, transparent-border,
float-alpha and independent numeric/golden assertions remain unchanged.
The native workspace command now passes with its existing lockfile: **130 tests**
across recorder/raster, including two new readback contract tests. Strict package
Clippy passes. Decisions: N1/N2 bound native read cost; N3/N6 preserve pixel
semantics. The earlier whole-path/macOS performance gates remain open.

The existing release `ops` benchmark on this shared Linux runner reports median /
p95 **71 / 177 ns** for one-pixel reads from dense 1080p. The same run's dense
full-frame read remains **21.445 / 44.732 ms**, and copy-base full output is
**0.897 / 1.106 ms**. These are native-only diagnostics under build contention,
not quiet-window/paired Electron timings or a complete EXT-RASTER-1 pass; the
dense full-frame cost still exceeds the 4-ms whole-path target.
