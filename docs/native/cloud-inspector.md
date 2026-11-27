# Cloud inspector integration

## 2026-11-05 — owned panel wiring

Branch `feat/na-cloud-inspector`, behavior reference
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. Linux only; native presentation
remains provisional until macOS verification and Opus review.

The Properties header now routes its menu through the existing asynchronous
`effectMenu` shell service, then submits the chosen definition to the real
kernel actor. Row menus similarly use the existing `contextMenu` service.
Cancelled menus, retired document/selection leases and disabled actions do
not edit. Effect Controls native parameter fields now write through their
registered binding and current model projection; formerly they fell through
the Properties-only binding route. Effect diamonds use the published playhead
rather than time zero. Registry refresh discards stale presentation metadata
without deleting authored effects. Effect field handles include document identity.

Properties publishes existing Weight, Align, independent-corner, parenting,
matte-source, 3D, motion-blur, Skew and Scale-link controls where applicable.
Independent Corners seeds all corners from the current radius atomically.
Settings publishes the specified Color Depth choices and submits the existing
project-root command. Invalid depths refuse; Undo restores the previous tree.

Decisions and selecting rules:

- Kernel receipts remain the success boundary; no optimistic document writes
  or synchronous JS on the UI thread (N3/N7/N8).
- Menu results retain their originating selection/document, and effect bindings
  require registration plus current applicability and editability (N3/N4).
- Preserve absent/unknown fields and missing effect definitions; retire only
  presentation metadata whose registry consumer is gone (N4/N6, prune rule).
- Use the approved Settings/Properties specs and existing menu host operations;
  no new look, copy or gesture is inferred (N1, Opus design ownership).

`DESIGN-TODO(opus)`: blend/matte menu descriptors and the previously recorded
structured/contributed/variable-font-axis controls remain placeholders. Existing
font, fill, point-pad and expression native services still need their shell
handlers; the shared kit's missing recipes remain visible.

Remaining application blockers outside this lane's ownership:
`app/src/builtins.rs` still leaves Project import/relink/download/preview effects
unhandled and does not handle Inspector font/fill/point/expression services.
`app/src/viewer2.rs` and the viewer-tools/kernel transform host must connect
Anchor Point press/drag with evaluated world/local transforms, rollback and
pointer cancellation. The shell pasteboard route must arbitrate text/local
clipboard/Finder/image ownership and dispatch durable import/placement.
These are not closed by model tests or Linux execution.

Verification in progress; final counts and merge base will be appended.

## 2026-11-05 — remaining owned bridge work and qualification boundary

The worker-side `Builtins::admit_media` entry now connects the existing
`Catalog::prepare_import` completion to the actor. It accepts only the owning
project/document/revision, retains the caller, returns the actual pending
receipt, preserves unknown metadata on relink and leaves decoder readiness
unknown. A native media service must durably retain bytes/old versions **before**
calling it. It is not a picker, file importer, pasteboard reader or byte-store
admission service, and no current shell caller is claimed. The regression
uses synthetic service-completion descriptors, not decoded media.

Viewer snapshots reuse the accepted chrome projection until the model stamp
or viewport changes. Re-querying identical state no longer invalidates a native
control's ticket. Changed state/viewport still retires it. Effect/Properties
writes require an acknowledged binding registration and matching document and
selection; collapse/dispose removes that capability. Shared selection arrays
avoid copying a large selection once per field. Opaque effect controls remain
read-only in both the view and admission path.

The native row-menu bridge now inserts a separate separator entry, preserving
the following action. The shell's existing flat menu still lacks the specified
grouped/Recent effect submenus and checked-state projection; those require
`app/src/v4_native.rs`/`app/src/builtins.rs` work by their owners.

Additional decisions: reuse unchanged immutable publications and share selection
leases (N1/N2/N7); keep native service completion distinct from file/decoder
readiness (N3/N4); preserve explicit unsupported/read-only controls rather than
claiming their native recipes exist (N1/N3, Opus ownership).

### Blockers, with required owner changes

- `app/src/builtins.rs`: execute Project `PickFiles`, `PickFolder`, `Import`,
  `Download`, `Reveal`, preview and media-placement requests. Wire durable
  service completions to `Builtins::admit_media`, then publish new decoded
  observations through `update_media`. The kernel/profile owner must supply
  byte-store admission for new originals/archived versions, including portable
  save/reopen; metadata alone does not supply bytes.
- Native clipboard/shell owner: connect `Edit > Paste` with text-focus ownership,
  local layer/key/effect clipboard precedence, pasteboard image/Finder items,
  durable import, placement and cancellation. No ownership was bypassed here.
- `app/src/viewer2.rs`/viewer tools and kernel owner: connect actual Anchor Point
  hit-testing, selection narrowing and the same pointer drag to evaluated parent
  transforms and guarded live edits. The actor's Selection/Gesture requests do
  not expose an atomic expected-document guard; checking a publication before
  enqueue is insufficient across replacement. Do not use whole-project Restore
  per pointer sample as a substitute for a proper live-edit path.
- Kernel captions owner: `commands/captions.rs::plan` still returns
  `NotImplemented` for both Add/Edit. `ContentCommand::Set` explicitly refuses
  captions. S088 styling cannot become working controls solely in the Inspector
  lane. Extension content also explicitly refuses in `commands/content.rs`.
- Kit/native host owners: multi-line text/expression fields, point-pad unit-slot
  and per-axis diamond recipes, font facts/picker, colour/fill/gradient services,
  graph focus/reveal routing, and generated/contributed Inspector fields remain.
  These include U02 and the rest of U13; no generic replacement UI was invented.

`DESIGN-TODO(opus)` remains the neutral placeholders for structured/contributed/
variable-font controls and missing kit recipes. Existing captions, point, font,
fill, expression and menu specs apply; they do not need a new design invented by
this lane. Blend/matte menu descriptors remain unimplemented placeholders.

No U02/U03/U08/U13/U14 row is fully closed: U08 has real V8/menu/actor coverage,
U13 has real Color Depth and scalar parameter coverage, while screen/native
service qualification and the blockers above remain explicit.

## 2026-11-05 — final merged Linux verification / BLOCKED

Fetched and merged `origin/feat/native-engine` at
`5bcd425ddc158cbd891ee4b515cf9993fad29a66` (already an ancestor; no conflicts).
Implementation commits: `2702a18cd` and `85f03e014`. This lane's full requested
acceptance remains **BLOCKED**, for the explicit shell/kernel/kit dependencies
above. The partial implementation is pushed for integration; no complete
U-row, whole-app parity or macOS behavior is certified.

All shell commands use `source /workspace/.tools/activate.sh; cd /workspace/powermove`.
The initially missing tool setup was restored outside Git. Rust 1.96.1 and Bun
1.4.4; no new dependency, workspace member or lockfile update was needed.
After the merge, the following exact checks passed:

```sh
cargo test --locked --manifest-path apps/native/Cargo.toml \
  -p pm-panel-inspector -p pm-panel-media -p pm-panel-viewerchrome \
  -p pm-builtins -p pm-effect-controls-wire -p pm-project-panel-check
# 302 passed, 0 failed, 0 ignored: inspector 110, media 72, viewer 61,
# builtins 35, effect-controls adapter 11, project adapter 13.

cargo clippy --locked --manifest-path apps/native/Cargo.toml \
  -p pm-panel-inspector -p pm-builtins --all-targets -- -D warnings
cargo fmt --manifest-path apps/native/Cargo.toml \
  -p pm-builtins -p pm-panel-inspector --check

cd apps/native/js/v4
bun test panels/inspector/test panels/effect-controls/test \
  panels/project/tests panels/viewer/tests panels/settings
# 230 passed, 0 failed, 89 snapshots (including 21 Inspector snapshots).
bun x tsc --noEmit -p panels/inspector/tsconfig.json
bun x tsc --noEmit -p panels/effect-controls/tsconfig.json
bun x tsc --noEmit -p panels/viewer/tsconfig.json
bun x tsc --noEmit -p panels/project/tsconfig.json
bun x tsc --noEmit -p panels/settings/tsconfig.json
bun x prettier --check panels/inspector/contracts.ts \
  panels/inspector/session.ts panels/inspector/test/menus.test.ts
```

TypeScript and Prettier passed on the same implementation before the final
integration fetch; the fetch introduced no source change. Secret scanning and
`git diff --check` pass before each commit. All Rust test invocations used
`--locked`. The existing native manifests/locks remain unchanged.

Mac verification: build the actual app, exercise header and trailing Add Effect
via pointer/keyboard, row menus and dismissal, numeric/choice/toggle/text/colour
native field dispatch, selection/document replacement during menus and edits,
Undo/Redo, AX/IME/text-focus ownership, light/dark/Increase Contrast, and Opus
screen review. Also verify the added Color Depth setting against real render
precision. Full Finder/image paste, point pads, anchor drags, durable media
import/relink/save/reopen and captions remain blocked on the owner changes above;
Linux metadata/actor tests do not prove those workflows.
