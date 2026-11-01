# P2 package 11: recorder canvas and CPU rasterizer

`apps/native/crates/recorder` (`pm-recorder`) implements a windowless Rust
Canvas2D command recorder. It contains no Node, Chromium, V8, WebKit, GPUI,
AppKit or GPU dependency. This package supplies native primitives; it is **not
production v1–3 Canvas/DOM binding certification**. The runtime must bind these
primitives, enforce origin/permission rules and keep existing JS object wrappers.

## Boundary and ownership

A host assigns each `Canvas` a unique, stable `u64` id. `key()` combines that id
with a monotonic content version. Reusing ids for another canvas is forbidden.
`commands()` exposes command kind, source key, geometry, transforms, pen,
compositing, alpha and clip count for headless drivers. State setters and path
construction update the recorder immediately. Draw commands snapshot that state;
`flush` rasterizes the unconsumed suffix synchronously. Pixel demand flushes
before returning. `CpuRasterizer` also replays owned command batches on an
independent worker and emits ImageData. Nothing changes a synchronous operation
into a Promise.

The host owns each recorder on its extension worker; there is no internal thread,
actor, main-thread callback or JS call. The kernel remains the document single
writer. A recorder does not modify projects, filesystem assets or history. The
host should `compact()` between consumed batches; it retires commands while
preserving float pixels, state and path. There is an explicit 65,536-command
batch admission limit and a 67,108,864-pixel allocation limit (including
8192×8192 canvases). Exceeding a limit
returns `Capacity`, without reducing resolution or deleting authored state.

`SourceKey { id, version }` identifies immutable sRGB/unorm8 input pixels.
`Sources::insert` refuses conflicting content for a key and refuses retired-key
reuse. Each command and pattern retains an `Arc<ImageData>` of the captured
version; repainting, transferring, resetting or dropping its producer cannot
alter that version. Source retirement removes the lookup, not consumer leases.
A self-draw must snapshot before recording, exactly like any other drawImage.
External decoded assets must receive new versions when content changes.

No additive API changes to `pm-vector` or `pm-text` were needed. Existing
geometry/render consumers are retained. Both dependencies remain in their
existing engine workspace.

## Implemented primitives

- Drawing state stack; postmultiplied CTM with translate/scale/rotate/set/reset;
  current default path is separate from saved state. Path points capture the
  CTM when constructed; strokes apply the pen in the current coordinate space.
- Move/line/close; quadratic and cubic Béziers; arc/ellipse in both directions;
  tangent arcTo; rect; 1–4 elliptical roundRect radii, common fit scaling and
  signed extents. Negative radii return typed spec errors; nonfinite path
  arguments are ignored before radius validation. Nonfinite state setters and
  invalid CSS colors retain the old value.
- Nonzero/evenodd fills and clips; miter/bevel/round joins; butt/round/square
  caps; miter limit; odd dash duplication and offsets; path/stroke hit tests.
  fillRect/strokeRect/clearRect do not modify the default path. A degenerate
  one-dimensional strokeRect is an open capped line.
- Solid float CSS Color 3 paint via pm-vector and in-gamut Color 4/5
  conversion via pm-text; live aliased gradient/pattern
  handles and immutable draw-time paint snapshots. Linear, radial and conic
  gradients; stable duplicate stops, straight color/alpha interpolation;
  pattern repetition in both axes, transform updates, wrap-filter taps and
  analytic nonrepeat boundaries. Gradient/pattern coordinates use the CTM at
  rendering, rather than the CTM at creation.
- All 26 Canvas composite strings: eleven Porter–Duff/add operations, eleven
  separable blends and hue/saturation/color/luminosity. Copy/source-in/
  source-out/destination-in/destination-atop affect transparent source outside
  the shape inside the clip. globalAlpha scales paint; clear ignores it and
  compositing while retaining CTM/clip behavior.
- Canvas text uses pm-text font resolution, HarfRust shaping, explicit Unicode
  bidi base direction, unhinted glyph outlines, ASCII whitespace normalization
  and one-line layout. It supports style/weight/size/family shorthand (quoted
  families, absolute units, em/rem/percent, optional ignored line-height),
  measureText, start/end/left/right/center, six text baselines, strokeText and
  maxWidth glyph condensation. It does not use editor wrapping, anchoring or
  the editor rasterizer's hard-coded baseline. The resolver is injected;
  `TextEngine<R>` works with native or Linux synthetic resolvers. The em box
  stays one font-size high even when the hhea box is larger. Static OpenType
  BASE records supply script-specific hanging/ideographic positions and em top;
  missing records use deterministic synthesis. `FontContext` supplies element
  and root sizes and inherited weight for relative sizes and bolder/lighter;
  invalid font assignments retain the previous font.
- Normalized drawImage source/destination rectangles support all 3/5/9-argument
  binding forms. Negative extents do not flip pixels; out-of-source clipping
  adjusts destination coverage. Nearest or bilinear reconstruction uses
  premultiplied source pixels and clamps drawImage filter taps to its crop.
- Fresh/copying ImageData, signed readback extents, transparent out-of-bounds
  pixels, dirty putImageData including negative dimensions and overflow-safe
  clipping. Reads ignore CTM, clip, alpha and composite; writes copy pixels
  ignoring all drawing state. The default is straight sRGB/unorm8; conversion
  unpremultiplies float pixels and rounds ties to even. Hidden RGB at alpha
  zero becomes transparent black. Same-size resize/reset clears pixels,
  current path, saved states and drawing state. Clone is blank; bitmap transfer
  consumes pixels while retaining drawing state and path.

## Rasterization and promotion

pm-vector resolves fill rules and expands stroked geometry in local space,
then flattens at its existing `1e-4` device-pixel tolerance. Clip intersections
use i_overlay's i64 polygon engine. The CPU backend integrates polygon edge
area over each pixel analytically, accumulating f64 difference rows and emitting
f32 coverage. Curves therefore have bounded flattening error; analytic coverage
is exact for the resulting polygons up to float arithmetic, rather than exact
integration of Bézier equations. Coverage is never an 8-bit supersample mask.
The independent test reference clips polygons to individual pixel squares and
computes shoelace area.

Clips intersect geometry before coverage. Repeated clipping by a partially
covered edge does not multiply its coverage and darken it. Compositing integrates
three regions: shape intersect clip, clip outside shape, and outside clip.
Paint is evaluated at pixel centers. Canvas default compositing is in sRGB code
values, with premultiplied float storage; this is distinct from the engine's
linear HDR working space. It is not an HDR Canvas implementation.

The backing uses 16×16 float tiles. An exact, full-canvas, unclipped drawImage
`copy` at identity and alpha=1 retains the immutable source as the base; only
subsequent changed tiles become float. This is a lossless representation change,
not a new rendering algorithm or a cache hit on a stale version. Generic draw,
clip and composite paths remain available. Full RGBA readback copies the base
and converts changed float tiles; transparent hidden RGB is canonicalized.

`PromotionTarget::present(&PromotionSnapshot)` is a synchronous host hook.
The snapshot includes id/version, pixels, current drawing state/path and saved
states. The target must preserve the JS canvas/context wrappers and retain or
copy the snapshot as needed. Only successful presentation marks the promoted
version. Failure leaves pixels, commands and state available for retry.
`import_promoted(expected, pixels)` accepts edits only at the exact current,
successfully promoted version; stale edits fail before they overwrite anything.
The Linux target double exercises success, failure, held snapshots, saved state
and conflicting native edits. This is not a WebKit DOM promotion implementation.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Plain worker-owned Rust recorder, synchronous readback, no UI dependency | N7 inspectable/headless; N8 main never waits on JS |
| Immutable source versions and draw-time paint snapshots; handles remain live | N3 exact authored intent; N4/N6 held references and sync contract |
| Reuse vector paths/text shaping; retain all existing consumers | N1 pro geometry; prune rule: remove only after last consumer |
| One-em text box, explicit font-style source and static OpenType baselines | N3 Canvas text intent instead of editor/Blink heuristics |
| Admit 8192×8192 sparse canvases without full float-frame allocation | N1 pro resolutions; N4 explicit capacity errors preserve state |
| Float paint and analytic polygon area, geometric clip intersection | N1/N2 quality before imitation; N3 correctness |
| Retain exact source as base plus changed float tiles | N1 performance without changing authored results; N3/N8 versioned native pixels |
| Use authored fractional font size without Blink's 1/100-px floor or UI font-cache seeds | N3 plus quirk policy: Chromium quantization/cache history is not recorded authored intent |
| Typed unsupported/error boundary rather than silently approximate unimplemented features | N3/N4/N6; the host must resolve the fallback before admitting those calls |
| Keep style-source colors and out-of-gamut mapping as explicit host gates; avoid detached Chromium system-color tables | N3 spec and authored context; quirk policy |
| Spec error ordering for restricted gradient numbers; prune zero-length stroke segments | N3/N6 spec geometry and observable errors |
| Conflict-check promoted edits and preserve native work on promotion failure | N3/N4 no lost edits; N6 object/state contract |
| Do not edit shared lockfiles outside lane ownership; isolated locked harness | Lane ownership and independent integration; no consumer removed |

Known departures from Chromium are intentional: there is no paint quantization
before compositing, coverage does not reproduce Skia's 8-bit edge artifacts,
and text does not inherit a renderer's unrelated font-cache optical-size seeds
or floor authored font sizes. The reference is the HTML Canvas specification,
not screenshots from Electron. Browser-dependent antialiasing is not treated as
saved intent. Existing engine compatibility APIs themselves are unchanged.

## Validation and integration gates

Run with Rust 1.96.1 from the repository root:

```sh
source /workspace/.tools/activate.sh
cargo test --locked --manifest-path apps/native/crates/recorder/isolated/Cargo.toml
cargo clippy --locked --manifest-path apps/native/crates/recorder/isolated/Cargo.toml --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/recorder/isolated/Cargo.toml --check
cargo run --release --locked --manifest-path apps/native/crates/recorder/isolated/Cargo.toml --example ext_raster_1
```

The isolated manifest points to the same production sources and tests. Its
lockfile starts from the engine lockfile to keep shared dependency pins. The
native-workspace `crates/*` glob already discovers the production manifest.
The requested `cd apps/native && cargo test --locked -p pm-recorder` was
attempted and correctly refuses to change the shared native lockfile. The
integrator must regenerate `apps/native/Cargo.lock` to add this package and its
engine dependencies. This lane does not own that file.

The synthetic benchmark runs 30 warmups and 100 frame samples: four changed
16×16 layers with a cubic path, versioned nested source snapshots, an exact
1080p base draw and full RGBA output. It reports native recording separately
from native raster plus readback. **JS and whole production path timings are
null**: no V8 callback, runtime binding, GPU upload or presentation is measured.
These numbers must never be described as a full EXT-RASTER-1 pass. Release
results and final test totals are recorded below after the validation run.

Remaining compatibility gates (do not silently route/drop these): full CSS font
shorthand including stretch/caps/system styles; out-of-gamut CSS color mapping and
style-source/system colors; font loading and project font lifecycle;
variation-aware (BASE device/variation deltas, MVAR) baseline/em metrics;
COLR/sbix/bitmap color glyphs (currently `NeedsPromotion` before recording text);
letter/word spacing and the additional text style properties in C-MATRIX;
shadow/filter and focus-ring APIs; explicit P3/float16/alpha=false contexts;
Path2D object wrappers; browser source decode/security/taint and ImageBitmap
close/transfer wrappers; PNG/JPEG and toBlob/toDataURL scheduling; V8 sandbox
backing-store allocation; actual remote-DOM creation/parser/clone/popout identity
and WebKit promotion. When a font has no applicable BASE records, baselines
synthesize hanging at 0.8 ascent and ideographic at descent from primary hhea
metrics. Nondefault variation deltas remain a cutover gate. Hosts must keep these
gates visible; this package alone cannot claim no-user-left-behind cutover.

On macOS verify native font matching/fallback/variations and font lifecycle,
actual V8 worker bindings and sandbox-owned pixel buffers, sync nested raster
continuations, retained JS canvas/context/paint references across real promotion,
remote-DOM edits and conflicts, alpha/color against the spec, extension corpus,
GPU upload and presenter behavior. Measure actual JS per changed layer p95 ≤1 ms
and complete EXT-RASTER-1 frame p95 ≤4 ms on supported Macs. Linux cannot certify
these gates or claim a GPUI/WebKit/Metal presentation result.

References: [HTML Canvas](https://html.spec.whatwg.org/multipage/canvas.html),
[Compositing and Blending Level 1](https://www.w3.org/TR/compositing-1/),
[c-matrix](c-matrix.md), [S5](s5-compat.md), [architecture](p0-decisions.md),
[engine service](engine-service.md).

### Linux verification, initial implementation

- 76 recorder spec tests pass with the isolated engine-aligned lockfile; no
  ignored recorder tests. This includes independent replay on another worker,
  analytic polygon reference pixels, source/paint lifetime, real pm-text font
  shaping, sparse tiles and promotion success/failure/conflict checks.
- `cargo fmt --check` and package clippy on all targets with `-D warnings` pass.
- Unmodified engine dependencies: pm-text's 41 unit tests and pm-vector's 68
  unit tests pass. Four of five pm-vector legacy geometry integration tests
  pass; **`paths_match_vector_paths_ts` fails** on this Linux x86_64 runner:
  `points of path1`, actual digest `2735fae539268e85`, fixture
  `93f367b1fabb1185`. No engine source/fixture/lockfile was modified. The
  platform-specific digest mismatch is an integration follow-up, not a green
  existing-suite claim. CoreText corpus/registry/Blink-size harnesses explicitly
  skip on Linux. Selecting `--lib --tests` avoids the existing pm-text WASM
  example's missing-level2-feature build error in the minimal-feature run.
- Release synthetic native result before dependency-pin alignment: Rust
  recording p95 **0.002024 ms/layer**, native raster plus full readback p95
  **1.718287 ms/frame** (100 frames / 400 layers). The aligned-lockfile rerun is
  recorded in the follow-up below. JS and the whole production path are
  unmeasured; neither target is certified by these native-only results.


### Final Linux verification

**92/92 recorder spec tests pass**, with fmt and all-target package clippy
(`-D warnings`) clean. Added assertions cover one-em metrics versus a larger
hhea box, static BASE hanging/ideographic coordinates, style-source relative
sizes and weights, case-insensitive font grammar and zero size, 8K sparse
backing, stroke hit-test boundaries, restricted gradient number error ordering,
in-gamut CSS Color 4 paint/stops and explicit style-source/wide-gamut color gates.
All three generated geometric fonts reproduce byte-for-byte; the generator
fixes font timestamps and never reads an installed or project font.

Final release benchmark with the engine-aligned dependency pins, Linux x86_64:

| Native synthetic measurement | Samples | p95 |
|---|---:|---:|
| Rust recording per changed layer | 400 | 0.002328 ms |
| Native raster + full 1080p unorm8 output | 100 | 1.839465 ms |
| JS recording | Not measured | null |
| Whole production raster path | Not measured | null |

The native synthetic row is below 4 ms. The JS ≤1-ms and complete production
≤4-ms gates remain open. The unmodified engine integration failure and required
native-workspace lockfile regeneration above remain unresolved by this lane.

`Error::Type`, `IndexSize`, `Range` and `Syntax` are typed primitives for binding
appropriate JS/DOM errors. Setter rejection retains state where the spec requires
it. `NeedsPromotion` and unimplemented-font `NotSupported` are **internal host
gates**, not permission to expose an invented JS exception for a valid Canvas
call: the host must finish the synchronous fallback at that boundary. This
package does not supply that production runtime fallback. Drawing zero-length
line segments follows the spec's tracing/pruning step, including round caps.
