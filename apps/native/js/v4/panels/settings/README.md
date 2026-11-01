# Settings v4 extension

Worker-side Settings content and controller. This is an integration candidate,
not a completed native Settings window or a visual acceptance claim. The only
production imports are Preact, the v4 renderer/host client and the UI kit. Test
fixtures contain synthetic data. No Electron, Chromium or Node runtime is added.

## Integration

Mount `settingsExtension.component` in the separate Settings NSWindow; do not dock
it or mount it as a sheet. Supply a `SettingsModel`, native text/select leases
for `bindings`, and the enqueue-only `SettingsWindow` adapter. The shell owns the
singleton window, saved position, minimum pm-design window size, height-only
resizing, title, native sidebar material, scrolling observations, search focus,
announcements, key routing and editor focus restoration. Feed section observations
to `trackSection`; manual scrolling cancels any host scroll animation. Host actions
must check generation/document identity at consumption, not only at enqueue.

`hydrate` reads the existing profile keys through `HostClient.settings.get`.
`write` uses `HostClient.settings.set`, and changes its displayed value only after
an acknowledgement. Unknown authored values remain intact. The current
`pm_services::SettingsService<SettingsHook>` is the existing persistence boundary;
no second durable store or speculative kernel command is created here.

Other settings model capabilities do not exist in pm-services on this base.
Implement `SettingsServices.perform` against the owning model services. It must
return a newer acknowledged `Publication`, use project transactions for authored
edits (including Undo), and check the supplied revision. Publish initial and live
rows, project identity, account/subscription states, extension variables and speech
models. Leases are host-owned, not channel names manufactured by the panel. The
controller rejects stale snapshots and in-flight results from a replaced project.
`perform` is a typed adapter seam, **not a registered v4 host command**.

Confirm account/extension deletion and speech removal with NSAlert, default Cancel,
then archive; cancellation must not publish success. No direct deletion API exists
here. Keychain/provider API keys must use a native secure-field lease that never
publishes the secret to this tree or ordinary settings storage. Secure rows stay
semantic placeholders until that API exists. Stop publications, dispose the model,
release leases and retire its host generation when the realm is destroyed.

The V8 worker performs search/filtering, including extension details. Previous
results remain until the next worker result and superseded queries are discarded.
Supply the native virtual-list viewport in logical indices to `viewport` (not its
overscanned range); it renders one viewport above/below, keeps a stable first-row
ID across publications, and carries total count. Native AXRowCount/row navigation
and the full-height scroller still need the owning host implementation.

## Spec coverage

| settings.md section | Implemented here                                                                                                              | Outstanding                                                                                        |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| 1 Form              | Separate-window descriptor; one continuous detail tree; section navigation/title/close effects; no Done                       | Actual NSWindow, source-list material, saved position/geometry, native scroll                      |
| 2 Model             | Typed publications; per-key async acknowledgement; project gating; active section/query/matches; service actions by ID        | Service implementations, native bindings and live subscriptions                                    |
| 3 Anatomy           | Existing kit controls, neutral selection, token references, semantic groups/labels                                            | Settings-specific form/source-list/search recipes and missing tokens                               |
| 4 Project           | Exact listed labels/help; disabled semantic rows without a project adapter                                                    | Live composition, custom Width/Height, full presets, export capability model                       |
| 4 General           | Exact rows/options/defaults and legacy profile keys; restoration capability gate                                              | Switch recipe, density model/key, workspace-to-agent handoff implementation                        |
| 4 Accounts          | Copy and disabled capability placeholders; published rows can supply status/actions                                           | Cloud/subscription/provider services, signed-in composition, secure Keychain leases, confirmations |
| 4 Transcription     | Download/progress/cancel/installed/remove/error/retry from typed model                                                        | Real speech service, confirmations and system ring/indicator rendering                             |
| 4 Extensions        | Search/type filters; bounded virtual rows; in-place details/back; variables; archive/Finder/Store effects                     | Native switch/icons, full AX/scroller virtualization, service implementation                       |
| 5 States            | Wire snapshots for every listed state; drag/drop and multi-select N/A                                                         | Native hover/ring/disabled painting; exact loading timing/spinner placement                        |
| 6 Keyboard          | Local sidebar navigation; search/Escape hierarchy; Cmd/Ctrl comma; Cmd F/W; root stops editor propagation; focus order tested | Shell preflight suppression/key equivalents, actual focus restoration and native traversal         |
| 7 VoiceOver         | Semantic labels/groups, selected state, help descriptors and search announcements                                             | Host help projection, virtual count/row navigation, real VoiceOver                                 |
| 8 Appearance        | All supplied visuals refer to pm-design; system behaviors delegated                                                           | Materials, contrast cards/borders and reduced-motion scroll verification                           |
| 9 Copy              | Spec copy only; data labels supplied by models                                                                                | Missing-copy questions below                                                                       |
| 10 Don't            | No Done/Apply, editor gear, checkbox/switch substitution, orange selection, or web export                                     | Opus review of the integrated window                                                               |

## DESIGN-TODO(opus)

1. Confirm the missing Settings-only Switch, secure field, minimal NSColorWell,
   grouped form, regular push button, source-list and search recipes after the
   owning UI-kit lane adds them. Current neutral rows deliberately omit unsupported
   controls; labelled `ToolbarItem` actions are provisional, not a push-button
   styling decision. Do not ship them as final Settings UI.
2. `metrics.sheet_inset`, `metrics.sheet_value_column` and a token-expression path
   for card radius/row minimum/section spacing are absent. No substitute numeric
   values or alternative spacing are selected; semantic groups remain unstyled.
3. Specify density's model/persistence key. The specified Normal default is shown
   disabled; no guessed profile key is written.
4. Supply approved copy for failed profile reads/writes, a filtered extensions list
   with no matches, and empty/unavailable transcription. Errors without approved
   row copy are inspection-only `failedIds`; no transport error is shown to users.
5. Complete composition preset labels hidden behind the spec's ellipsis, and
   labels/help for Width/Height and unsupported composition-depth reasons. These
   remain model placeholders; no dimensions, format defaults or copy are invented.
6. Resolve exact row loading placement against shared README §3 (which disallows
   per-row spinners), and define empty-list icon/progress-ring component inputs.
   Current loading uses the kit's existing ProgressRow, provisionally.
7. Specify search-match run highlighting through the kit (accent_wash and semibold
   text run, not whole-row highlighting), and search semantics/count presentation
   while extension type filtering is active. Search works; run painting is absent.

## API gaps outside ownership

- UI kit: all recipes in TODO 1, token math, native search placeholder/progress,
  a noninteractive labelled form group, status symbols/actions, per-row help.
- Surface AX: `aria-description` is retained on wire but currently not projected
  as AccessKit help; structural virtual-list lacks List role/AXRowCount and native
  scroll-to-row. Metadata here describes the requirement, not an implementation.
- Host: settings publications/subscription and typed settings-service operations;
  projectless settings currently needs a host-provided identity because HostClient
  requires a nonempty project ID. Do not invent a project just to open Settings.
- Services: project/export capability adapter; account/subscription/provider,
  transcription, extension enabling/archive/Finder/Store, workspace agent handoff,
  density persistence and native secure-field/Keychain path.
- Shell: separate-window lifecycle, source-list semantics, native key suppression,
  native binding leases/change dispatch, scroll/viewport and actual focus effects.

## Rule-selected decisions

| Decision                                                                                            | Rule                                                                             |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Profile writes use existing keys and only acknowledge accepted values                               | N3 authored intent; N4 preserves migrated user preferences; kernel single writer |
| Preserve unknown values and retain all old panels/compatibility paths                               | N4/N6, prune only after no consumer remains                                      |
| Use injected acknowledged service/window interfaces; no speculative command registration            | N3/N7 honest by-ID inspection; N8 no UI-thread JavaScript wait                   |
| Omit unsupported controls and styling rather than substitute inspector checkboxes/plaintext secrets | Design ownership; N1/N2 pro-grade gate; N3 exact intent                          |
| Bound extension rendering and preserve viewport identity                                            | N1/N7, shared README §4                                                          |
| Archive-only semantic actions with native confirmation owned by services                            | N4 archive, never delete; N6 preserve extension data                             |
| No Done button, neutral sidebar, no web export                                                      | settings.md §1/§10 and owner decisions (HIG placement, N3)                       |

## Validation

From `apps/native/js/v4`:

```sh
bun install --frozen-lockfile
bun test panels/settings
bunx tsc --noEmit -p panels/settings/tsconfig.json
bunx prettier --check panels/settings
bun test
bun run typecheck
bun run build
```

Wire tests use the real shared Rust six-op and kit validators and snapshot their
accepted trees. Native hover/focus snapshots explicitly retain the same op tree:
these states belong to system controls, so Linux does not simulate their painting.
No performance certification follows from bounded row-count tests. Verify native
light/dark, contrast/transparency/motion variants, real controls, AppKit text/IME,
clipboard/dictation, AX help and counts, keyboard isolation/focus, 60k scroller and
VoiceOver navigation, actual service persistence/Undo/archive/Keychain, lifecycle
fencing, no main-thread JS wait, and Opus screenshots/sign-off on macOS.

## Linux merge validation — 2026-11-01

Merged `origin/feat/native-engine` at
`4204a153dbf8af916ef2b92d7ca01858e363516a` and
`origin/feat/na-v4-integrate` at
`e5e209928817cb9434823ebf6ddf7d64f2213441`. The Cargo.lock conflict was resolved by
retaining the four incoming panel-model entries alongside the existing v4/viewer
entries; no package version or checksum changed. No new crate or dependency is
introduced by this panel.

- `bun test panels/settings`: 51 passing tests, 18 accepted-tree snapshots.
- `bun test`: 223 passing tests, including the 51 settings tests; no skips/failures.
- `cargo test --locked -p pm-services -p pm-surface`: 170 passing tests
  (75 services, 95 surface); no ignored tests or failures.
- Both TypeScript configurations, panel/shared Prettier checks, shared browser
  build and standalone Settings browser bundle pass.
- `cargo clippy --locked -p pm-services -p pm-surface --all-targets --no-deps -- -D warnings`,
  `cargo fmt -p pm-services -p pm-surface -- --check` and
  `rustfmt --edition 2024 --check js/v4/panels/settings/validation.rs` pass.
- The panel-owned AccessKit test executable is compiled with `rustc -D warnings`
  against the locked validator dependencies; it does not modify shared sources.

These are merge/build results. They do not resolve the service/host API gaps or
replace macOS verification and Opus review listed above.
