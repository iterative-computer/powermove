# Gate policy

The native engine is a correct renderer that stays backwards compatible, not a copy of
today's WebGL output (plan amendment 22). So a gate never requires byte equality with a
capture of today's app. It proves two things.

1. **Authored intent, exactly.** What the project says stays what it says.
2. **Quality at least today's.** Against a high-precision reference render of the same
   document, the engine is at least as close as today, and the preview equals the export.

When today's output is measurably wrong, the engine fixes it and adds a row to
[quality-ledger.md](quality-ledger.md). The captures stay. They are "today" in every
comparison and the witness for authored intent.

## Gates that stay exact

pm-eval dual-answer evaluation (1e-9); text layout (line breaks identical, advances,
font seeds, census); retime bit for bit; the pm-vector native/wasm conformance digest;
extension compat specs (`engine-extension-compat`, `engine-compat-moat`,
`engine-compat-context`, `engine-customization-corpus`, `engine-private-setup-compat`);
protocol fixtures; 3D picks and bounds; export stream properties (`verify.rs`).

## Pixel gates

`pm_reference::compare` puts every pixel in one class. Errors are the largest channel
difference in 8-bit code values (CV).

| Class | Pixels | Rule |
|---|---|---|
| Interior | ≥ 2 px from any edge, no resampling | Within 1 CV of today (authored colors, blends, mattes) |
| Edge | Partial coverage, grown by 2 px | Engine max, p99.9 and mean error each ≤ today's + the precision bound; per pixel, engine error − today's error within the previous bound |
| Resampled | Drawn from a resampled source | As Edge |
| Unreferenced | The reference does not model it yet | The gate's previous bound against today |

Precision bounds (`Bounds`): 0.5 CV for 8-bit frames (`EIGHT_BIT`), 2⁻¹¹ ≈ 0.125 CV for
half-float frames (`HALF_FLOAT`). The previous bound (`per_pixel_cv`) is ≤ 1 CV for 99.9%
and ≤ 4 for any pixel at 8 bits, ≤ 1 at half float: no single pixel may regress under
today's worst error elsewhere. A gate with other storage states its own. Geometry is
exact: alpha area (relative) and centroid (px) within 1e-3 (`same_geometry`). Preview ==
export: the paused viewer's frame and the export's are identical (`identical`).

A case where today is closer to the reference than the engine fails. Report it; never
widen a bound to pass it.

Each interior pixel in G-C2 and the non-text regions of `text_corpus` retains
legacy equality within 1 CV, or proves a correction directly against the authored
f64 value: within 1 CV of it, with error exceeding today's by at most the storage
precision bound. This alternate proof applies only to pixels outside the original
legacy bound; a correction elsewhere never tightens an already passing pixel. It proves the solid radius even where today's ignored radius
leaves a flat, wrong corner far from the corrected contour. Original capture
distances remain in the report; geometry and authored paint are still exact.

## Owned pending reference gaps

When a stricter reference exposes an unmodelled effect or compositing defect,
`pm-render/tests/fixtures/corpus-pending.json` records its anonymous frame ID,
owner (L6 effects or L4 compositing), failed numeric checks and a digest of all
measured engine, capture and reference samples. These entries are reviewed
individually; recording observations does not admit them or pass the gate.

Every pending frame is still rendered and measured. A changed measurement, new
failure, missing full-run measurement or resolved entry fails for owner review.
Only failures wholly in unreferenced pixels, including the existing 3D class
until L4 supplies its reference, are eligible. Whole-frame text composition
uses the same ratchet only when its authored text-only projection matches and passes the paired isolated
text capture at the unchanged 2 CV bar, or every differing text pixel lies
in the independent mask-edge band. Missing paired evidence stays blocking;
the isolated measurement is included in the pending checks. Projection follows
the capture converter and ignores only generated effect-keyframe transport IDs;
text values, layout, masks, animators, effects and retained transforms stay exact. Authored geometry,
preview/export identity, render errors and modelled vector coverage remain
blocking. Pending entries never change a global bound.

## The reference

`engine/crates/pm-reference` (tests only) computes in f64 from the standards and the
authored formulas, never from the engine's rendering or color code: exact supersampled
coverage of outlines (`coverage`), Porter-Duff and W3C blending (`composite`), the exact
Gaussian (`gaussian`), sRGB, BT.709 and BT.1886, PQ, HLG, H.273 matrices, ranges and
primaries, and ICC matrix/TRC profiles with optional unclipped conversion (`color`).
`tests/engine_formulas.rs` cross-checks it against pm-color.
`pm-render/tests/support/reference.rs` builds a document's reference frame: authored
values from pm-eval, all math from pm-reference. It models backgrounds, 2D solids,
vectors, motion-blur coverage and masks. Supported SVG paths, gradients, normal
source-over groups and clips use direct f64 paint and coverage on a shared 16×
grid, resolved per image before layer compositing. Other SVG constructs use an
independent 32× resvg reference whose RGBA8 paint quantization is a stated
limitation; it is not a converged f64 paint reference. Content it cannot model is unreferenced
over its complete affected footprint, including transparent effect input pixels.

`tests/corpus.rs` (G-C2) is the model conversion; its gated frame must equal the export
frame, and the paused preview must equal that export. Video selects from source timestamps
on every surface, holding the last source frame without browser seek padding.

## Who converts what

Each lane extends `support::reference` for its content, converts its own gates, and
deletes the today-equality test it replaces.

| Lane | Gates | API |
|---|---|---|
| L2 text | G-T1, G-T2, G-T5 | `coverage::fill(outline, _, 32, …)` on glyph outlines at the exact layout positions; `composite::blend`; `gaussian::blur` (σ = blur × world scale); `compare`, `same_geometry` |
| L3 vectors, masks, SVG | G-V1, G-V2, G-V3 | `coverage::fill(_, _, 16, …)` for shapes, paths, SVG; `gaussian::blur` with `Edge::Transparent` at σ = max(0, feather)·\|world x\|/2; `compare`, `same_geometry` |
| L4 compositing, sampling, viewer, 3D | G-C3, G-C4, G-C5 bars, G-D1, G-D2, G-M1 | `composite::{blend, porter_duff, luma709}`; `Class::Resampled` with a declared-kernel reference (move the one in `tests/sampling.rs` here); `support::reference::preview_and_export`, `identical`; groups, precomps, mattes |
| L5 stills | G-I1 | `color::{Transfer, Primaries, MatrixTrc, convert, bradford}`; images as `Resampled` |
| L6 effects, export | G-E1, G-X1 | `compare` with stated f32 bounds; `color::{rgb_to_ycbcr, dequantize_luma, dequantize_chroma}`; `identical` |

Removed with this policy: `tests/half_store.rs` (G-C1), the FONTS-R ratchet (G-T3) and
`level1_keeps_todays_sampling` (G-C5).
