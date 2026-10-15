# pm-edit: geometry-dependent edit kernels

Plan: docs/rust-engine.md section 6 ("Making every write visible", item 4),
section 5 rule 4 with amendment 1, section 26 Phase 1 ("geometry-dependent edits
switch to pm-wasm behind a flag"), appendix C ("Align, group, comp bounds;
model-group bounds"). Builds on pm-eval's layer questions
(docs/engine/pm-eval-layers.md: `world_pose_values`, `aim_values3d`,
`pose_values3d`, `group_bounds`, `bounds`, `text_layout`).

## What it is

The renderer stays the authority for the document and Undo. A kernel reads
pm-wasm's mirror, computes what today's TypeScript edit would write, and returns
it as canonical patch operations (contract 5's JSON form). The renderer applies
them to the live document inside the caller's history transaction, so Undo and
the edit log see an ordinary write. Kernels never write the mirror: the applied
operations reach it through the write barrier like any other edit.

A kernel works on an `Edit`: a scratch copy of the document with its own
evaluator. Writes land in the copy as they are made, so later reads see them, as
in the TypeScript code (a reparented parent before its children, a parent's
baked keys before the child sampled against them). `Edit::finish` diffs the
touched compositions into the smallest operation list (`pm_model::ops::diff_ops`):
leaf sets where a value changed, id-keyed inserts and deletes where layers or keys
came or went.

| Module | Ports | Kernel |
|---|---|---|
| `pose` | `parenting.ts` | `reparent` (`preserveParentPose`), `preserve_world_pose`, `preserve_world_aim`, `apply_pose_values` |
| `groups` | `layer-groups.ts` | `move_to_group`, `ungroup` (rebase, or bake when anything is animated), `group_placement` (a new group's pivot) |
| `align` | `extensions/inspector/align.ts`, `AlignmentStrip.svelte` | `align`; `distribute` (new, see below) |
| `anchor` | `font-anchor.ts`, `size-anchor.ts`, `editing.ts` `runOne` | `capture_font_anchor`, `capture_size_anchor`, `anchor_reference`, `anchor_moved` (Pan Behind) |
| `split_text` | `split-text.ts` | the layers a split by characters, words, or lines creates |
| `comp_bounds` | `compositions.ts` `leaveAttributes` | Pre-compose › Leave all attributes: size and anchor shift |
| `materials` | `editing.ts` `synchronizePeers`, `scene3d/materials.ts` | shared shader channel copied to every peer surface, in every composition |

Pose-preserving edits are built on `Scope::world_pose_values`, `pose_values3d`,
and `aim_values3d`, as pm-eval-layers.md recommends. `Pose2d::rotation` is a
`JsValue` (a layer collapsed to a point keeps its stored rotation unconverted);
`apply_pose_values` takes `JsValue`s and keeps JavaScript's `+`, `-`, `*`, `/`
semantics, so a stored string concatenates as it does today.

Every value keeps the TypeScript operation order, so results equal the legacy
edit bit for bit wherever the inputs are the same floats. Where the legacy code
throws, the kernel throws the same message (user-facing `Error`s) or a
`TypeError` (engine-internal shapes the UI never reaches).

## Interfaces

**Rust.** `Edit::new(document, comp, open, expressions, ids)`; each kernel takes
`&mut Edit` and layer indexes of the edited composition; `Edit::finish()` returns
the operations. `IdSource { base, next }` makes ids `prefix + base + n`.

**pm-wasm.** `edit.<kernel>` requests through `query` (crates/pm-wasm/src/edit.rs):
`{ op, comp, open, t, uid: { base, start }, …arguments by layer index }`. Replies:
`{"ok":{"ops":[…],"next":n,…answers}}`, `{"throw":{kind,message}}`, or the
query fallbacks `{"host"}` (three.js geometry: 3D objects and model groups until
Phase 3c), `{"fonts"}` (fonts not served yet), `{"ink"}` (glyph ink without
outlines). Answers: `ids` (ungroup), `bounds` (groupPlacement, anchorReference),
`moves` (align, distribute), `plans` (splitText), `w h ax ay` (leaveAttributes),
`count` (syncPeers).

**Renderer.** `engine-doc/edits.ts` (`NativeEdits`, `PM.EngineDoc.edits`),
behind the native evaluator flag (`engine-doc/flags.ts`):

| Mode | Behavior |
|---|---|
| `off` | `PM.EngineDoc.edits` is null; every call site runs the TypeScript edit |
| `on` | the kernel's operations are applied (`doc.write`), the TypeScript edit does not run; a kernel throw is rethrown with the same class and message |
| `audit`, `audit-proxy`, `strict` | the TypeScript edit runs and is what the document keeps; the kernel's operations applied to the shadow must give the same compositions (dual answers under `edit.<kernel>`) |

Before asking, the open composition's layers (and `comps` for material peers)
are marked written and the barrier flushes, so the mirror holds every write of
the transaction so far. Unanswerable requests run the TypeScript edit and are
counted under `fallbacks` (`PM.EngineDoc.stats().edits`).

Call sites, each keeping its TypeScript implementation as the fallback and the
dual-mode reference:

| Edit | Site |
|---|---|
| Set parent (`set_layer`) | `parenting.ts` `reparentPose` |
| Group (pivot placement), Ungroup, Move to group | `layer-groups.ts` |
| Align (alignment strip) | `AlignmentStrip.svelte` through the `editKernels` service (`kernel/install.ts`) |
| Font and size anchors (capture, reference, Pan Behind rescale) | `font-anchor.ts`, `size-anchor.ts`, `editing.ts` `runOne` |
| Split text | `split-text.ts` (the kernel builds the layers; insertion, hiding, grouping stay in TypeScript) |
| Pre-compose › Leave all attributes | `compositions.ts` |
| Shader peer synchronization | `editing.ts` `synchronizePeers` |

## Time (section 5, rule 4 with amendment 1)

- Times a kernel does not change are echoed: the original `f64` is copied through.
- Times it creates or moves are computed in ticks on the composition's exact
  grid (`IngestRate`) and written with `IngestRate::seconds_of`: `n / fps` (the
  document's own float grid) on a frame start, `ticks / TICKS` off the grid. A
  composition whose rate does not resolve keeps the legacy float arithmetic.
- A key moved between layers (`t + from − to`, material peers) is echoed when the
  two starts are the same instant, else shifted in ticks, across compositions with
  different grids too.
- An ungroup bake samples the playhead and every frame of the composition; frame
  keys land on `n / fps` exactly as today. The playhead key, when the playhead is
  off the frame grid, lands on the playhead's tick.

## Ids and keys

Created ids are `prefix + PM.uid('') + '_' + n` (`k` keys, `L` layers, `ts` text
styles, `ta` text animators). The separator keeps them apart from every id
`PM.uid` makes (`prefix` + 7 random base-36 characters in the app, `prefix-n` in
tests) and matches the identifier rule of 3D materials
(`/^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/`). Keys a kernel creates are written in the
native ease model (`inInterp`/`outInterp`, `inEase`/`outEase` with linear
influence, `autoBezier: false`, `continuous: false`): the form the document holds
once evaluation has adopted `PM.KF`'s `eo`/`ei`/`hold` handles, which the app does
on the first evaluation of every track.

## Deliberate differences from today

1. **Exact grid for created and moved key times.** On a composition stored with a
   float NTSC rate (29.97, 23.976, 59.94), a key time that is a sum of frame times
   and an off-grid offset is computed on 30000/1001 rather than on the float rate:
   it differs from the TypeScript `n / 29.97 + from − to` by the float rate's drift
   (about 1e-6 relative). Frame-aligned results are identical (`n / fps`).
2. **Off-grid playhead keys.** An ungroup bake's playhead key lands on the
   playhead's tick (at most half a tick, 0.7 ns, from the float `time − from`);
   values sampled against a parent baked on the same tick move with it (observed:
   at most 1e-6 relative, 7 values in the generated vectors).
3. **Created keys in native ease.** As described above; equal after adoption.
4. **Distribute** has no TypeScript counterpart (the strip only aligns). The two
   outermost layers stay; the others move along one axis so the chosen edges or
   centers fall at equal steps, or so the gaps between neighbours are equal.
   Tested against that definition (`tests/distribute.rs`); no UI calls it yet.

Dual-answer mode compares with these allowances: key times within twice the
compared compositions' drift (scaled by the times and the largest layer start),
keyframe tracks adopted on both sides first (legacy evaluation adopts stored
handles as a side effect of the TypeScript edit), created ids by order of first
appearance, numbers inside rewritten expressions with section 25's tolerance.

## Tests and results

All measured on 2026-10-14 at this branch.

- **Generated vectors** (`tests/vectors/generate-edit-vectors.ts` under bun, the
  TypeScript edits on seeded random documents; `tests/edit_parity.rs` replays them
  with pm-wasm's production expression adapter): 1,717 cases (reparent 323, move
  to group 238, ungroup 180, group placement 164, align 236, size anchor capture
  166, anchor reference 27, anchor moved 27, leave attributes 89, material peers
  207, split text 60), 916,325 numbers, 896,977 bit-identical, max relative error
  9.5e-10; 3,159 key times in difference 1 and 7 values in difference 2; 257 error
  messages identical; 0 mismatches.
- **Corpus v1** (`PM_EDIT_CORPUS` over the 132 `loaded.json` of
  `v1-20261012-230504-ae47a3b3`, vectors written outside the repository): 2,038
  cases (reparent 416, move to group 443, ungroup 9, group placement 203, align
  467, size anchor capture 354, leave attributes 146), 2,269,537 numbers,
  2,267,949 bit-identical, max relative error 2.8e-14, 0 mismatches. Not compared:
  163 align cases on text (the bun harness has no canvas fonts), 103 cases on 3D
  objects and model groups (host-only in Phase 1), 3 align cases whose TypeScript
  bounds rasterize vector paths (no canvas in the harness). The corpus has no
  shared materials or split text; those kernels are covered by the generated set.
- **Pipeline** (`engine-doc/edits.test.ts`, vitest with the real pm-wasm module):
  the same edit script (reparent, unparent, group, move in and out, two ungroups
  with bakes, size anchor capture and Pan Behind, align, Leave all attributes, and
  shader peers across two compositions) run with the flag `off`, `on`, and
  `audit` leaves the same document (created ids and compositions normalized);
  Undo returns to the original; `audit` records 0 `edit.*` mismatches; every
  kernel answered natively.
- **pm-wasm** (`crates/pm-wasm/tests/edit.rs`): replies, id sources, throws, and
  the world pose kept through reparent and an animated ungroup applied back to the
  mirror.
- **e2e** (`--workers=1 --retries=0`, one Electron at a time,
  `POWERMOVE_ENGINE_FONTS=1` with the pm-bridge addon built): 23 specs (64 tests)
  covering groups, 2.5D groups and layers, alignment, font and size anchors, split
  text, compositions and pre-compose, viewer tools (Pan Behind), 3D scenes, text
  animators, and `native-edit-kernels.spec.ts` (the strip's Align left,
  unparenting, grouping, an animated ungroup checked at five frames, the size
  anchor's Pan Behind, the font anchor, split text through the layer menu, Undo to
  the start, and the kernel counters). Flag `off`, `on`, and `audit`: 62 passed,
  2 failed in each mode, the two failures the quarantined
  `ae-audit-implementation.spec.ts:9` and `precomp.spec.ts:46`, failing the same
  way with the flag off. `audit`: 97,983 dual answers compared across 66 page
  sessions, of them 30 `edit.*`; 712 audits, 0 failures, 0 mirror mismatches. The
  first run found 2 `edit.splitText` mismatches, both the comparison reading
  `PM.Edit.mutate`'s result object instead of its created layers (fixed; the
  split text, font anchor, and pm-edit specs rerun in `audit`: 0 mismatches,
  `edit.splitText` compared 2).
- **What fell back in the e2e runs**: `ink` (glyph ink, below), `fonts` (the first
  text question of a page, before the registry's answers arrive), and one `host`
  (a 3D group's bounds).

## Phase 1 limits

- 3D objects and model groups: bounds are three.js answers until Phase 3c; group
  placement, align, and Leave all attributes on them run the TypeScript edit.
- Text: split text, font anchors, and align on text need the font registry
  (`POWERMOVE_ENGINE_FONTS=1`); without fonts the TypeScript edit runs.
- Glyph ink: a text layer's raster selection (font anchor capture, group
  placement and align around text, split text's font offset) needs outlines, and
  the Phase 1 registry serves metrics-and-shaping subsets only
  (`pm_fonts_outlines` stays off), so those questions answer `{ink}` and run the
  TypeScript edit. The kernels themselves are parity-tested over given layouts and
  offsets; they switch on in the app when the text lane serves outlines.
