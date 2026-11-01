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
