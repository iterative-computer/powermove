# Quality ledger

Every place the native engine deliberately renders differently from today's app because
today's output is measurably wrong (QUALITY-1, QUALITY-2, [gate-policy.md](gate-policy.md)).
Authored intent never changes here. A row records a correction, the standard it follows,
and the proof.

## Row format

One section per area (for example `## Video`, `## Text raster`). Each section states once:

- **Reference:** how the f64 or supersampled reference is computed and the standard it
  follows (for example "H.273 and BT.2100 in f64", "exact coverage, 16 × 16 cells",
  "W3C Compositing and Blending §10 in E").
- **Gate:** the test that proves the engine is at least as close as today.
- **Units:** 8-bit display code values (CV) unless stated.

Then one row per correction:

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|

## Required evidence

- Both distances come from the row's gate, on the cases it names. Describe cases; never
  name private corpus projects.
- Before/after side-by-sides stay outside Git, in the lane's evidence directory, linked
  from the lane report.
- The corrected content renders the same in the preview and the export.
- A correctness fix needs no decision, even when it shifts every project's look. Only
  a correction where "correct" is genuinely ambiguous (no standard applies, or tags
  conflict) cites the orchestrator's decision.

Rewrite a row when its numbers change. Keep no round history here.
