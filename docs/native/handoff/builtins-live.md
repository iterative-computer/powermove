# WIP: built-in v4 panels in the native app

This is an implementation checkpoint for Jude, **not a completed or visually
approved integration**. Continue on `feat/na-builtins-live`; the base is
`origin/feat/native-engine` at `e493e794dc526d35b1fcade7e59af14d3a9599d7`.
Do not push the engine branch or main.

## Panel status

All nine production JSX entries mount in separate v4 V8 realms against a real
kernel actor. The application registers these exact shell destinations:

| Entry | Shell destination | Current status |
| --- | --- | --- |
| Timeline | `timeline` | Live v4 tree, native model window, native clip/keyframe painting and selection; transport seek/play routed. Graphs, gestures and several host effects remain incomplete. |
| Properties | `inspector` | Live v4 tree; native selection, transform/composition leases and text content. Native writes and stale-selection rejection tested. Presenter is incomplete beyond these rows. |
| Effect Controls | `layer-effects` | Live production extension; real Inspector effects adapter, typed kernel edits/acknowledgements, native menus. Needs visible selected-effect verification. |
| Viewer | `viewer` | Live production chrome; native viewer model and state changes. Existing engine Metal viewer remains responsible for pixels. Transparency/zoom fixes need visible verification. |
| Project | `assets` | Live production extension; native Catalog/ProjectBridge, guarded prepared edits and actual actor acknowledgements. Media readiness remains `UnknownRuntime`; many media host effects are not connected. |
| Effects | `fxbrowser` | Live production extension; reads kernel effect registry and selection. Add resolves an installed native definition and submits typed kernel commands. No fabricated registry entries are seeded. |
| Agent | `agent` | Live production extension and native agent panel/persistence model. Opens a GPUI popup window, **not yet the specified toolbar-anchored NSPopover**. Provider/native action execution is incomplete. |
| Settings | `settings` | Live production extension, host data subscription, actual profile settings commands and native controls/window. Project rows, accounts/providers and speech services are incomplete. |
| Export | `export` | Live production extension; typed options, destination picker and real engine child job/cancellation/reaping. PMV export and other items below remain incomplete. |

The old per-document temporary panel renderer and old PNG-only Export handler
have been removed. `app/src/panels.rs` retains shared native geometry/formatting
and recent-project/library navigation because those still have consumers.
Runtime creation currently requires an open project: before that, the library
and the shell's ordinary placeholder path remain. Settings/Agent without an
open project still need an application-level host.

## Hosting and ownership

- `crates/builtins/build.rs` bundles `js/v4/app/entry.tsx` with Bun. The executable
  embeds the bundle; there is no source-directory lookup or Node runtime at launch.
- `pm-builtins::Builtins` owns one `pm-runtime-v8::RuntimePool` per project window.
  Each built-in uses `RealmKind::V4Extension(panel_id)`. macOS enables the
  `macos-sandbox` feature and requires the S1 sandbox archive plus matching bindings
  described in `docs/native/runtime-v8.md`.
- JS `HostClient` requests pass through `HostRouter<HostDataService<Data>>`.
  `data.rs` reads immutable kernel publications, scopes bindings/settings to the
  realm, submits typed requests, polls real actor receipts and publishes replies.
  JS receives presentation projections, not a writable project authority.
- `properties.rs` issues native Inspector/V4Adapter leases. Selection/document
  changes retire them. Surface events validate generation and exact tree revision.
  Native field/control callbacks also check the current lease before dispatch.
- `models.rs` coordinates existing TimelineClient, Inspector, viewer chrome,
  ProjectBridge, Agent and export option models. Native platform effects use the
  window-owned queue in `app/src/builtins.rs`; unfinished actions explicitly refuse.
- Accepted `pm-surface` trees are consumed by `app/src/v4_view.rs`, resolving UI-kit
  recipes/tokens into GPUI. AppKit text, popup, checkbox, switch and push-button
  slots join GPUI's presentation/accessibility transaction. Native keyframe paint
  receives a Rust-owned scene; JSX carries only row identities.
- Shell docking, header drag/context menus, native toolbar and commands remain
  shell-owned. Authored headers are separated from panel bodies. Native keyboard
  prevent-default policy is applied before asynchronous JS delivery.

## Verification already run

The latest native command below passed **140 tests, 0 failed, 1 ignored**:
4 `pm-builtins`, 39 app unit/integration tests (including nine separate
mount/bind/kernel-change/AX tests), 2 app executable workflow/MCP tests, and
95 surface unit/integration tests. The new binding test writes Opacity to the
actual kernel, changes selection, and verifies the old lease is rejected.
Both light and dark UI-kit/AX preparation are checked by the nine app tests.
These are not a substitute for native window/VoiceOver testing.

The native-number JS tests passed **2/2**. The Inspector panel run has **5 passes
and 21 failing snapshots** after the approved SF Symbol metadata was connected;
functional tests passed. Review the actual changes and regenerate synthetic
snapshots, then rerun the remaining panel/kit suites. Do not silently accept
snapshot changes as proof of visual correctness. App entry TypeScript checking
passed; rerun it after further edits.

Local logs (not committed): `/tmp/pm-builtins-native-tests3.log`,
`/tmp/pm-builtins-number-tests.log`, `/tmp/pm-builtins-inspector-tests.log`.
Superseded test executables were deleted after the run to reclaim disk.

## Exact build and test commands

Use the pinned compiler explicitly; the default installed compiler differs.
After sourcing the environment, override debug info to avoid duplicate large
builds. The shared Cargo target can incorrectly appear fresh after another
checkout's build; refresh changed Rust sources **inside the build lock** if a
new token/API is reported missing despite being present in this checkout.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export RUSTY_V8_ARCHIVE=/private/tmp/pm-s1-v8-artifact-final/librusty_v8.a
export RUSTY_V8_SRC_BINDING_PATH=/private/tmp/pm-s1-v8-artifact-final/src_binding.rs
export DYLD_LIBRARY_PATH="$PM_FFMPEG_DIR/lib"
unset V8_FROM_SOURCE

diskutil info / | grep 'Container Free Space'
flock /tmp/pm-build.lock nice -n 10 cargo +1.96.1 test \
  --manifest-path apps/native/Cargo.toml \
  -p pm-native-app -p pm-builtins -p pm-surface
flock /tmp/pm-build.lock nice -n 10 cargo +1.96.1 build \
  --manifest-path apps/native/Cargo.toml -p pm-native-app
flock /tmp/pm-build.lock nice -n 10 cargo +1.96.1 build \
  --manifest-path apps/native/Cargo.toml -p pm-surface --examples

bunx tsc --noEmit -p apps/native/js/v4/app/tsconfig.json
export PM_SURFACE_WIRE="$CARGO_TARGET_DIR/debug/examples/wire"
export PM_UI_KIT_WIRE="$CARGO_TARGET_DIR/debug/examples/ui_kit_wire"
bun test --cwd apps/native/js/v4 app/native-number.test.tsx
bun test --cwd apps/native/js/v4 panels/inspector/test/panel.test.tsx
```

Set the two wire variables when running Bun panel tests: otherwise their preload
may invoke Cargo outside the shared build lock. Rebuild examples first so the
wire validators contain the current components/tokens.

## Remaining work, in priority order

1. **Re-open and inspect the latest executable.** Initial GUI inspection showed
   overlapping number wells, duplicate Project/Properties headers, an opaque
   Viewer over the Metal content, duplicate timecode and missing required symbols.
   Fixes are compiled/tested in this checkpoint but have not been rechecked on
   screen. Verify the new numeric AX actions and keyboard focus too.
2. Opening Export initially crashed because GPUI synchronously rendered a new
   window while Controller was already borrowed. `show_builtin` now opens from
   an asynchronous task after that update. This compiles but needs visible
   Settings/Agent/Export opening, closing and reopening tests. Prevent concurrent
   duplicate opens, verify project switching with floating windows, and replace
   Agent's popup window with the specified anchored NSPopover.
3. `v4_view.rs` is an incomplete six-op/native-kit renderer. Finish grid/media
   geometry, clipping, symbols/overlays, timeline graph and editing gestures,
   native source viewer actions and complete accessibility actions. Check 60k-row
   virtualization and scroll/focus retention; current native scene rows are bounded.
4. Finish Properties presentation/applicability (Content/Type/structured/time/layer/
   contributed rows, expressions, colour/font/pad services, scrub transactions).
   Header Add Effect/New Layer and row menus are not wired through the native
   `inspector` platform request yet. Alignment reaches typed kernel commands, but
   the existing kernel alignment implementation still refuses text bounds.
5. Finish native media effect handling in `builtin_platform` and replace
   `UnknownRuntime` with observed engine/media readiness. Only guarded document
   edits, open-composition/settings routing and some focus effects are connected.
   Timeline host effects beyond seek/play/frame/open composition also remain.
6. Connect Agent providers/native actions and Settings project/account/speech/
   extension services. Existing model errors/unavailable states are retained;
   nothing should claim a provider/network action completed when it did not.
7. Export: replace optimistic `exportCapabilities` in `Controller::facts` with
   actual probes/planner results; wire BackgroundTasks, portable PMV/original
   closure, presets, Reveal, correct caption count, exact range/frame summary,
   and all failure/cancel states. Current PMV execution explicitly errors and
   caption count is still zero. Do not ship those optimistic capability flags.
8. Prune dead compatibility formatting, old platform menu/field helpers and unused
   Controller scroll/edit/select members only after confirming no consumer remains.
   Resolve compiler warnings and refresh/review affected JS snapshots.
9. Complete native keyboard/menus/docking/VoiceOver checks and all requested light
   and dark captures before considering this integration complete.

## On-screen setup and screenshot status

**No PNG/JPEG screenshot artifacts were saved.** CUA screenshots were inspected
in-session only. Do not report `/tmp/pm-builtins/` as completed screenshot evidence.
The latest code has not had its final on-screen pass. GUI lock was released at
handoff; no task-owned preview app should be left running.

The synthetic project is `/tmp/pm-builtins/synthetic-project.json` (three layers,
position/opacity keys). Use `--profile /tmp/pm-builtins-profile`. Acquire
`/tmp/pm-shots/gui.lock`, check `/tmp/pm-shots/USER_ACTIVE` before and throughout,
and stop GUI work immediately if it appears. Save final captures outside Git to
`/tmp/pm-builtins/`: main light/dark, selected Properties, Effect Controls,
Project, Effects, Agent, Settings and Export.

The raw command-line executable was not discoverable by CUA as an app. A disposable
bundle at `/tmp/pm-builtins/Powermove Native Test.app` with identifier
`com.powermove.native.builtins-test` worked when its executable was an APFS clone
of the binary, **not a symlink**. Its old executable was removed for disk cleanup;
copy the new one before launching. Launch the exact bundle executable with both
profile and project arguments, then attach CUA to that bundle identifier. Calling
CUA getApp before the test app is running can launch it without those arguments.
Do not interact with the owner's separate Electron Powermove app.

```sh
# Only after acquiring gui.lock and checking USER_ACTIVE:
cp -c "$CARGO_TARGET_DIR/debug/powermove-native" \
  '/tmp/pm-builtins/Powermove Native Test.app/Contents/MacOS/powermove-native'
PM_NATIVE_APPEARANCE=dark \
  '/tmp/pm-builtins/Powermove Native Test.app/Contents/MacOS/powermove-native' \
  --profile /tmp/pm-builtins-profile /tmp/pm-builtins/synthetic-project.json
# Repeat with PM_NATIVE_APPEARANCE=light for the other appearance.
```

Maintain 5 GB Container Free Space. At handoff it fell below the floor during
concurrent shared builds even after removing this task's completed executables
and prior obsolete intermediates. Do not start more builds until space is back
above the floor. Keep screenshots, profile data and diagnostic logs out of Git.
