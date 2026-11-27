# Engine inputs for remaining native commands

Status: engine-only cloud lane, Linux. Reference: `v1.2.0`, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`. Kernel commands and platform hosts
are unchanged. This supplies their missing data/geometry inputs; it does not
claim their command handlers or the native cutover gate are complete.

## Public text measurement

`pm_text::measure` is available without the `render` feature or a renderer.
The `TextMeasure` trait accepts evaluated `compat::TextContent` and returns
`Result<TextMeasurement, MeasureError>`. It can be implemented by a headless
service/test double. The kernel remains the only project writer; measuring
never changes a document, registers extensions, waits for JavaScript, or draws.

- `measure_compat(&mut Compat<R>, content)` preserves the existing level-1
  wrapping, advances, baselines and grapheme boundaries over `FontResolver`.
  Resolver owners must check pending/missing answers and generation failures;
  the API cannot infer readiness from an arbitrary resolver implementation.
- `measure_level2(&mut Level2, content, spans)` uses the existing font-backed
  native shaping engine, including styles, axes, fallback and font line metrics.
  No registered fonts or uncovered visible clusters are explicit errors.
- `measure_level2_with_ink(..., font_by_run_key)` additionally measures glyph
  ink from full, single-face font bytes with `level2,ink` features. It reuses
  the engine's color glyph placement and unhinted outline/control-box policy,
  including normalized variation coordinates and synthetic bold/oblique.
  `ink` does **not** enable `render`, GPU, pixel drawing, or `vello_cpu`.
  Missing face bytes and invalid font/outline reads are errors.

The serializable result contains the unpadded union of line boxes, a separate
optional authored paragraph box, per-line text/UTF-16 ranges/baselines, optional
font ascent/descent (native level 2), caret positions and character/word/line
units. Ink is explicitly `NotMeasured`, `Empty`, or `Bounds`; layout/advance
boxes must never be substituted for measured ink. Level 1's existing authored
baseline is not relabeled as a font ascent. Units retain source indices and
shaping poses for Split Text and text animators.

Coordinates are layer-local, x from the alignment origin, y down. Caret
queries use UTF-16 offsets and reject positions inside a grapheme. Soft-wrap
positions can have upstream and downstream line placements; nearest-caret
queries compare visual positions, including RTL. No interaction padding,
selection styling, hit radius or other native design is introduced.

Choose the saved layout level explicitly. Existing level-1 ink/typography
anchor semantics still live in the renderer-free
`pm_eval::text_measure::CompatMeasure`; integration must retain that provider
for old projects, including its separate bitmap anchoring versus drawn ink.
Do not route old projects through native-level-2 geometry. The new ink helper
uses the same policy as the existing `pm-eval` level-2 consumer; that consumer
is retained until the integration lane adopts the public helper (prune rule).

Font discovery/registration remains the existing CoreText authority. Prepare
font bytes/answers for the kernel worker, pin their registry generation and
invalidate measurements when fonts change. Do not place a font broker or a
renderer on the UI thread's synchronous command path. Inputs must already
resolve animated typography, box sizes, styles and `fontAxis.*` at command time.

## Caption model and codecs

New engine workspace crate: `pm-captions`.

- Typed `CaptionCue`, `CaptionWord`, `CaptionStyle` and `CaptionsContent` have
  the v1.2.0 JSON field names. Content preserves unknown top-level fields.
  Project JSON round-trips preserve cue IDs, word timings and style exactly.
- Normalization sorts stably, cleans cue text, clips/clamps word timings,
  trims preceding overlaps and merges near-simultaneous speaker cues. The
  shipped minimum duration is 0.04 s, precision six decimal places.
- Pure insert/update/delete/split/merge/move/trim operations and binary-search
  lookups are available by cue ID. A text edit clears stale word timings;
  moving a whole cue carries its words. CJK tokens, punctuation, unspaced joins
  and character-mapped word timing follow the tag's model.
- `rebase_cues_for_start` keeps cues at their composition times through trim-in
  or a layer split. `clip_cues_to_end` clips the head, including word ends.
  Negative deltas extend the In point without moving the heard words.
- SRT/VTT import accepts BOM, CR/CRLF, missing numbering/hours, comma/dot
  fractions, VTT metadata/NOTE/STYLE/REGION, settings, markup, entities, ruby
  and inline timestamps. `ParsedCaptions::skipped` reports unreadable blocks.
- Sidecar writers implement offsets, range clipping and rebasing, millisecond
  timestamps, SRT numbering, VTT language headers and escaping. Both formats
  are display-text/time interchange; as in the shipped writer, they do not
  serialize project IDs, styles or exact stored word timings. Keep lossless
  project JSON and original imported bytes; a sidecar is not a project archive.

Times in this crate are **layer-local seconds**. The command facade still maps
composition-time command inputs to layer time, applies fit-duration policy,
locks, history, provenance and single-writer publication. Split-cue editing
and split-layer clipping are distinct operations.

`normalize_cues`, `normalize_captions_content`, `insert_cues` and
`parse_with_ids` accept an ID allocator. `split_cue` takes the tail's explicit
ID and refuses empty/colliding IDs. Default standalone parsers use import-local
`c1`, `c2`, … IDs; inserting them into a layer must use `insert_cues` or that
layer's allocator. Malformed allocator output gets deterministic suffixes,
never an unbounded retry loop or overwriting an authored ID.

No transcription, caption rendering, panel behavior or native visual presets
were invented. The six existing caption styles/presets are generated reference
data, not a new design proposal. Unicode casing uses the Unicode default;
there is no inferred host-locale typography choice.

## Effect definitions and defaults

New engine workspace crate: `pm-fx-defs`. `builtins()` exposes **16 effects,
4 transitions, 13 layer factory/default schemas**, PM.CH metadata and the
explicit inspector transform Reset schemas. The committed JSON retains exact
shader strings, flags, UI parameter metadata and default values, plus the
reference commit and source blob hashes.

`Definition::defaults`, `default_for` and `channels` supply native effect and
transition parameters. Fresh channels have `{v, kf:[], expr:null}` and own their
storage. Number/color/toggle defaults retain their JSON types. Unknown effect
or transition IDs return `None`; no replacement definition is fabricated.

`channel_default(type, path, DefaultContext)` uses the layer **factory**,
including type-specific origins, 3D channels and `c.` content defaults. Dynamic
sizes/positions resolve against the owning composition. Missing/non-finite
required dimensions cannot become null property values.

`reset_default(type, path, context)` separately uses the declared v1.2.0
inspector **Reset** transform table. In particular, resetting a solid's
position centers it even though its factory origin is zero. This is explicit
source behavior, not a Chromium rendering artifact. Content/effect/transition
rows use their active definition's supplied default; when none is supplied,
the shipped row clears animation/expression while retaining the static value.
A kernel Reset handler must preserve this distinction.

This is not a second contribution registry. Resolve the active extension
contribution before consulting a built-in fallback. Deserialize active
extension definitions into `Definition` (unknown metadata survives), or adapt
their parameters directly. Never replace a contributed default merely because
its ID matches a built-in. Dynamic font axes, shader uniforms, structured
properties and extension-layer parameters still use their own active schemas.

## Commands whose engine inputs are now available

These are exact handler/API names in the current kernel. "Unblocked" here
means the missing engine input is available; all existing admission, undo,
selection, lock and publication work stays in the kernel integration lane.

| Command | Available input | Integration still required |
|---|---|---|
| `LayerCommand::TransformLayers` / `transform_layers` | Measured text boxes/units; native effect/transition/default definitions | Port the higher-order capability compiler and dispatch its primitive commands. |
| `PropertyCommand::Reset` / `reset_property` | Explicit Reset table, active-definition defaults, fresh property channel shape | Resolve the channel's schema and remove keys/expressions through the normal transaction. |
| `EffectCommand::Add`, `Reset`, `SetTransition` / `add_effect`, effect Reset, transition edits | The 16/4 built-in definitions and typed defaults | Adapt definitions to the kernel's existing command types and preserve active overrides. |
| `CompCommand::Precompose` / Pre-compose | Text geometry/units for existing bounds kernels | Port the composition move/parent/timing/appearance compiler; use the saved text level. |
| `TextCommand::Split` / Split Text | Shaped character/word/line `GlyphLayout` units with source indices | Feed existing `pm_edit::split_text` and preserve posed glyph positions/IDs. |
| `TextCommand::AddAnimator`, `RemoveAnimator`, `SetStyleRange` | Units, UTF-16 ranges and styled native measurement; existing evaluator remains available | Implement actor mutations and preserve stored animator/style schemas. |
| `CaptionCommand::Add`, `Edit` / `add_captions`, `edit_captions` | Complete cue operations, style patches, codecs, clip split/trim normalization | Coordinate mapping, duration fitting, locks, undo/provenance and archival source retention. |
| `TransformCommand::CenterAnchor` / Center Anchor | Native font ink and separate layout/paragraph geometry; existing legacy ink provider | Use `pm_edit::anchor`, the saved level and typography anchor offsets without moving the picture. |
| `TransformCommand::Align`, `Distribute` for text | Measured text geometry for the existing alignment kernels | Replace the no-font refusal/provider, apply world/parent transforms and preserve authored anchor offsets. |
| `GroupCommand::Group`, `MoveToGroup` text pivots | Text geometry for union/group-placement bounds | Replace the no-font shortcut and run the normal group-pivot kernel. |

Neither these helpers nor `Rect::center` decide which geometry a command
should use. Keep the existing `pm-eval` selection/typography-anchor policy;
interaction padding is not an authored anchor or an advance box.

## Decisions and selecting rules

| Decision | Rule |
|---|---|
| Add pure engine APIs, no kernel writer or platform UI changes | N7 headless/inspectable, N8 no JS/renderer dependency; kernel single writer. |
| Keep layout level explicit; reuse registered fonts and existing shaping/ink policies | N3 exact authored geometry, N4 old projects, N6 UTF-16 contract. No new Chromium quirks are specified. |
| Return unavailable ink/fonts explicitly; never use character-count or advance bounds as glyph ink | N1 professional geometry, N3 correctness. |
| Preserve caption model semantics, six authored presets and CJK/word timing behavior | N3 authored text/timing, N4/N6 existing projects and extensions. |
| Keep original source/project serialization separate from lossy sidecar interchange | N4 archive-never-delete; N3 exact project data. |
| Repair colliding ID allocation without losing cues or looping | N3 correctness and N7 deterministic agent edits. |
| Escape literal SRT markup/entities; clip rebased negative intervals; accept hour fields beyond 999 | N2/N3 correct round-trips and format behavior over shipped data-loss/parser limitations. These differences have dedicated tests. |
| Preserve explicit inspector Reset behavior separately from factory defaults | N3/N6 declared source contract; an app Reset table is not a browser rendering quirk. |
| Active contributed definitions override immutable built-in fallback data | N4/N6 extension contract; no silent built-in substitution. |
| Retain old JS modules and existing engine ink consumer | Prune rule: both still have consumers. |
| Gate the existing WASM example's level-2 export on its feature | N7/N8 renderer-free minimal build must compile; default builds keep the same exports. |

## Linux verification and remaining platform work

`engine/Cargo.lock` is outside this lane's explicit ownership list. The new
workspace members require two package entries (18 added lines; no dependency
version changes). The exact integration change is supplied as
[`workspace-lock.patch`](../../engine/crates/pm-captions/tools/workspace-lock.patch).
The results below used that prepared local lockfile change. The integrator must
apply/include the patch in the workspace lockfile before `--locked` verification
of the pushed tree; the lockfile itself is deliberately excluded from this lane's
commits under the explicit ownership constraint.

Pinned Rust 1.96.1. The combined engine run has **103 passing tests**:
**55 pm-text (41 existing feature-enabled units + 14 measurement tests),
42 pm-captions (31 reference/round-trip tests + 11 edge/invariant tests),
6 pm-fx-defs**. The caption oracle replays **133 reference operations**,
including CJK fixtures and **30 inline subtitle strings from all six files**
that contain timestamp/arrow fixtures in the pinned desktop source. The tag
has **zero standalone SRT/VTT fixtures**. Every parsed fixture round-trips
through both sidecar formats and lossless project JSON. The generator checks
source-file coverage and refuses to proceed if an inline fixture is uncaptured.
Repeated full fixture arrays are shared in the corpus without changing inputs.

All fixture data comes from the public synthetic repository tests, never user
projects. Both generators verify the exact reference commit and reproduce their
committed output under `--check`. Node 24 is development-only oracle tooling;
neither new Rust crate executes JavaScript or needs Node at runtime.

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
# Required integration step while Cargo.lock remains outside lane ownership:
git apply engine/crates/pm-captions/tools/workspace-lock.patch
node engine/crates/pm-captions/tools/generate.mjs --check
node engine/crates/pm-fx-defs/tools/generate.mjs --check
cd engine
cargo test --locked -p pm-text -p pm-captions -p pm-fx-defs --features pm-text/ink
cargo test --locked -p pm-text --no-default-features
cargo clippy --locked -p pm-text -p pm-captions -p pm-fx-defs --all-targets --features pm-text/ink --no-deps -- -D warnings
cargo fmt -p pm-text -p pm-captions -p pm-fx-defs --check
```

Strict Clippy, formatting and whitespace checks pass. `pm-text` also passes
without default features, including its compatibility-only measurement tests;
no painter is linked. The repository's CoreText registry/corpus/Blink-size
harnesses print macOS-only skips on Linux, and the existing macOS-only level-2
suite has no runnable Linux tests. These are not counted as executed parity.

Still verify on macOS: real CoreText/project-font discovery and invalidation,
full/split collection faces, fallback and color glyph ink/anchor parity,
variable and synthetic faces, complex-script/IME editing, native viewer/export
geometry, and command-to-undo integration under actual kernel scheduling.
The existing S1/S5, GPUI/WebKit and rendering/performance gates remain open.

No native visual/interaction/copy choices were made, so there are no new
`DESIGN-TODO(opus)` placeholders. Future visual output remains provisional
until the Opus design review. No kernel, app, vendor, spike or private-plan
paths were edited by this lane.
