# Export built-in v4 panel — 2026-11-01

Status: **worker implementation and Linux contract tests; NOT application cutover
or visual approval**. Source design: `docs/native/design/panels/export-sheet.md`
§§1–9 and `panels/README.md` (including all four owner decisions). Behaviour
oracle: shipped v1.2.0 export-defaults.ts/exporter.ts; W069/B424–445/B504/A264.
No old panel, extension or user state was removed. All visual output remains
provisional until Opus review.

`index.tsx` exports `mountExportPanel`, `ExportSheet`, `ExportButton`, and the
inspectable `ExportModel`. The host supplies the existing v4 `HostClient`, native
six-op transport, accepted anchor and unique subscription ID. Query/subscribe
`project` with channels `["export"]` must return `ExportSnapshot` from the native
model adapter. Capabilities and registered, generation-owned binding handles are
required before opening. There is no loading screen or JS document store.
The returned root supports normal v4 inspect/drive-by-ID. All interactive IDs
start `export.`; kit-generated kind IDs remain in the accepted tree.

The thin Rust `pm_promedia::export_panel` adapter provides typed lossless options,
effective delivery intent, typed `pm_protocol::ExportSettings` mapping and the **remember → destination → work** sequence. Native planned size/range/colour/precision survive the mapping; exact rates, straight alpha, preview requests and original-only delivery are enforced.
`ExportHost` is the platform seam. Linux doubles prove order, errors, snapshot-ticket forwarding, PNG folder/PMV writer routing and no work on picker
cancel. They do not run an encoder or an NSSavePanel. The application must supply
the trait implementation using pm-kernel, pm-promedia jobs and engine-service.

## Spec section → status

| Section                 | Done                                                                                                                                                                                                             | Not done / integration gate                                                                                                                                  |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| §1 Model                | Three kinds, six formats; remembered format per kind; field gating; rational NTSC rates; draft validation; caption/preset/run state                                                                              | Native project projection, bindings and planner implementation; legacy archive ingestion                                                                     |
| §2 Anatomy              | Exact copy, conditional rows/cards, PMV body, custom aspect, summary/note, action labels and footer order in the accepted tree                                                                                   | Exact form geometry, card recipe, wide sheet, capsule treatment and default NSButton appearance require missing kit API/tokens; neutral placeholders remain  |
| §3 Destination/progress | Native command boundary; Rust ordering and destination-kind validation; actual progress/ETA; cancel/retry/close; done/cancelled close; next export resets                                                        | Actual AppKit picker, frozen engine job, caption sidecars, resource preview publication, completion banner/notification/Finder; no end-to-end delivery claim |
| §4 States               | Wire snapshots for empty, selected, disabled, custom error, capabilities, choosing, all progress/failure/terminal phases; hover/focus tree invariance                                                            | System hover/focus painting and disabled-format subtitle rendering; loading, drag/drop, multi-select, 60k rows are N/A per spec                              |
| §5 Keyboard/focus       | Both export shortcuts after shell arbitration; kind arrows; Return/default; preset Return isolation; Esc settings-only; ⌘. progress; native validator asserts depth-first focus order including conditional Save | Shell registry/menu hookup, initial focus resolution to selected radio, native focus restore/trap and input-driver/IME walkthrough                           |
| §6 VoiceOver            | Real AccessKit dialog/radio/control roles, selected state, exact labels, numeric units/progress; full descendant counts                                                                                          | Row help, progress value description/ETA, polite summary and done/failed announcements are requests in metadata; kit does not project them yet               |
| §7 Appearance           | Only existing pm-design tokens in actual styles; 16 scheme/accessibility combinations validate; no scan animation                                                                                                | Exact sheet/card/footer/preview layout, appearance captures, contrast and Opus approval on Mac                                                               |
| §§8–9 Copy/don't        | Spec copy; no Code/web choice; no fake preparation percentage; system-placement SheetAnchor; destination before engine work                                                                                      | Real system segmented/pop-up/checkbox/push controls and one default accent require renderer integration                                                      |

The test-only wire inspector is a native workspace member. It replays real atomic
commits through Surface and Panel, and returns actual AccessKit roles/values.
The 21 checked-in snapshots record the entire accepted op tree. An N/A empty
state has no sheet; it does not synthesize product UI. Appearance tests certify
valid token references/semantics, never actual native painting or layout.

## Host/API gaps — exact obligations

These are **required application APIs**, not commands already implemented by
HostDataService. That service forwards authorized commands to KernelData; it does
not implement the export backend. This lane does not edit surface/src or js/v4/src.

- `["export"]` project projection: `version:1`, monotonic `sequence` independent
  of document revision, nullable project facts, options, presets, capabilities,
  run and already registered draft bindings. Reject retired project/generation
  work; publish progress even at the same document revision. Do not expose a
  mutable document to JS. Unknown legacy settings stay archived/preserved.
- `export.plan {options}` returns a complete validated projection using the
  Rust engine planner's ticks, range/endpoints, size and still timecode. The
  worker ignores obsolete plan acks and keeps Export disabled until planning
  catches up. Disabled formats carry engine reasons verbatim.
- `export.begin {options,destination:{kind,suggestedName}}`: reserve the project
  export slot, call the Rust adapter, acknowledge actual kernel persistence,
  present the nested NSSavePanel/NSOpenPanel, then freeze the job intent/snapshot
  and use `ExportHandle::start_scheduled`. Engine output must verify and publish
  without clobber. Original media only; do not silently omit captions, extension
  producers or media. PMV uses the portable writer with original-media closure.
  Picker cancellation returns Idle and leaves every setting intact.
- `export.preset.save {name,options}`: app-wide kernel persistence, fresh stable
  ID, preserve opaque fields, retain/archive same-name predecessors. No path or
  output name in options. Return the acknowledged projection.
- `export.cancel`, `export.closeFailure`, `export.retry`, `export.reset` return
  acknowledged projections. Cancellation must release/reap the worker before
  publishing Cancelled. `export.reveal` brings its owning window forward.
  Late command acks cannot regress a newer export observation sequence.
- The native owner publishes leased preview handles and real frame/total/elapsed
  and ETA; uses pm-promedia BackgroundTasks with TaskKind::Export; and performs
  spec completion banners (including hover timing), inactive notifications,
  Finder actions and accessibility announcements. The UI neither fabricates
  completion effects nor owns timers for native banners.
- UI kit needs `SheetFormRow`/`SettingsGroup`/`PushButton`, full-width system
  segmented-control mode, capsules, wide-sheet/preview geometry and token math.
  It currently only has inspector rows and toolbar actions. FormRow/Group are
  neutral structural wrappers; ToolbarItem actions explicitly request NSButton
  via metadata. SheetAnchor still uses its existing sheet_width recipe. No
  literal substitute is added for `metrics.sheet_width_wide`, `sheet_inset`, or
  `sheet_value_column` (absent in this base). Existing recipe overrides would
  require changes in the prohibited surface/src and js/v4/src paths.
- Typed kit/AX support is needed for option subtitles, placeholder, help,
  live announcements, progress value-description/ETA, default buttons, initial
  radio focus, modifier-aware default prevention for ⌘. and sheet dismissal policy. Ordinary periods in text fields are never suppressed. `metadata.export` records requests;
  it is not interpreted by the current kit, and tests never claim it is.

## DESIGN-TODO(opus)

1. **Retry intent:** specify whether Try Again reuses the failed frozen job and
   destination or opens destination/settings again. The panel delegates
   `export.retry` and makes no local choice.
2. **Failure outside rendering:** approve presentation/copy for failed planning,
   preset persistence and ambiguous transport acknowledgements. For now these
   remain inspectable faults and disable Export; there is no invented toast.
3. **Custom size accessibility:** approve distinct width/height control names.
   The copy table only supplies `Size`; both fields currently use that label and
   expose pixels, retaining the axis in inspectable metadata.

Missing kit recipes/tokens above are API gaps, not requests to redesign the
specified sheet. No colour, dimension, type, layout or product copy is invented.

## Rule-selected decisions

| Decision                                                                                                  | Selecting rule                                                     |
| --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| No Code/web UI or command path                                                                            | Recorded owner decision; spec §§1,9                                |
| Rational 24000/1001 etc behind the specified decimal labels; native planner owns timing                   | N3: preserve exact authored timing, avoid copying browser rounding |
| Preserve dormant options and unknown fields; PMV ignores hidden raster limits; retain all prior consumers | N4/N6 and prune rule                                               |
| Await real host acknowledgements, serialize commands, fence faults, ignore stale planning/observations    | N3/N7: inspectable truth over optimistic success                   |
| Native worker owns picker/encoder; UI only enqueues; platform services behind ExportHost                  | N7/N8 and kernel single writer                                     |
| Retain straight alpha only for specified formats; no silent proxy/unsupported-resource fallback           | N1/N3 and engine export-safety contract                            |
| Neutral wrappers/metadata where the kit cannot express decided design; no substitute metrics              | Design ownership and N1; exact appearance remains an explicit gate |

## Verification

From `apps/native` (after the required activation):

```sh
cargo test --locked -p pm-promedia -p pm-surface -p pm-export-panel-wire
cargo clippy --locked -p pm-promedia -p pm-export-panel-wire --all-targets -- -D warnings
cargo fmt -p pm-promedia -p pm-export-panel-wire --check
cd js/v4
bun test
bunx tsc --noEmit -p panels/export/tsconfig.json
bun run typecheck
bunx prettier --check panels/export/*.ts panels/export/*.tsx panels/export/*.json panels/export/README.md
bun run build
```

No ffmpeg, GNU shell features, temp-path assumptions, /proc or macOS-only tests.
The worker bundle also executes in an isolated test realm without process,
Buffer, document or window. Test helpers import only the real shared v4 API/kit
and owned synthetic fixtures; there are no private plan or spike dependencies.

macOS gate: implement the APIs above, then exercise real destinations, ProRes/
H.264/VP9/PNG/PMV delivery (including captions/audio/alpha and originals), cancel
and retry resource cleanup, concurrent editing/export, native sheets/control
metrics, keyboard/IME, full VoiceOver/help/announcements, light/dark and all
accessibility settings, 1×/2× hit rectangles, no UI-thread JS waits and Opus review.

## Linux evidence — 2026-11-01

- 11 new export-adapter tests; pm-promedia totals 94 tests. pm-surface totals 95;
  the combined native command passes 189 tests, with no skipped tests.
- 47 export JS tests, 21 accepted-tree snapshots; full v4 suite 219 tests.
- Strict Clippy for the touched crates, rustfmt, both TypeScript checks, panel
  Prettier and browser builds pass. The dedicated whole-panel browser bundle is
  executed without Node/Chromium/DOM globals by the suite.
- Merged native-engine `4204a153dbf8af916ef2b92d7ca01858e363516a` and retained
  v4-integrate `e5e209928817cb9434823ebf6ddf7d64f2213441`. The only merge
  conflict was the lockfile's added panel-package list; all additions survived.
  The lane's own lock changes add the test inspector and pm-promedia's protocol
  dependency without upgrading packages. No engine lockfile change is needed.

## Frozen-contract reconciliation — 2026-11-01

The export lane now merges native-engine freeze
`f7844ebc007c17900cb2d65f4ef63c6bd3f8b005` (p0-decisions.md: integrated
headless eval 40/40). Its v4-api.md, ui-kit.md, surface implementation and JS API
match the already consumed v4-integrate contract. No export implementation
adaptation or lockfile update is required; no surface or JS API source is edited.
The export lane's 189 native tests, 219 full-suite JS tests (47 export tests),
21 snapshots, strict Clippy, rustfmt, TypeScript, Prettier and browser builds
were rerun against this merged base and pass.

The host/API gaps above remain **additive API requests**, not changes to the
frozen contract: an export projection/command adapter and registered bindings;
sheet form/group/push-button recipes and specified tokens; and typed native
support for option subtitles, placeholders, accessibility help/announcements,
progress value descriptions, initial focus, sheet dismissal and modifier-aware
shortcut defaults. Native engine/picker/resource/completion wiring and the three
DESIGN-TODO(opus) questions remain application/design integration gates. The
40/40 API eval does not certify this panel's native appearance or delivery.

Rule selection: consume the frozen API unchanged (orchestrator freeze, N4/N6);
request additions without editing another lane (ownership); retain explicit
macOS and design acceptance gates (N1/N3).
