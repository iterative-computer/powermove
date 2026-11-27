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

## Cloud UI foundations checkpoint — 2026-11-05 (`feat/na-cloud-ui2`)

This section supersedes the earlier checkpoint's foundation status, **not its
uncompleted Mac/Opus acceptance gates**. The lane is not a completed application
integration. Do not turn Linux model tests into an on-screen or provider pass.

Merged inputs: built-ins `e835967d12f944b4a6b2fba011783c0cdc903d1b`, engine
`ed5482622f41c533199a724dcb8c8738dd4dcc93`, timeline renderer
`ce10963fae2eb4467492e2083b9aa15e29d9ddf6`, surface follow-ups
`934532ab53b0fd4f8bbe7c76efb269611c8f85c4`, and updated engine/specifications
through `78294c4c93cdb2a3a18aef8004fc42f9e5e420a9`. Renderer reconciliation keeps
the relocated media bridge, additive approved tokens, and one declaration/AX
mapping per kit component. No parallel input branch was modified.

The behavior reference is now **main
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`**. Old projects/extensions remain
required. Opus's `main-delta-ui.md`, `3d-studio.md`, and updated panel specs are
merged. Per the owner's latest routing, **U rows and the new 3D interface belong
to the follow-on UI lane**, not this implementation. P/E rows stay with the
respective delta lanes.

### Implemented and directly testable foundations

- Application Agent/Settings can mount without opening a fictitious document.
  Application publications refuse document commands even if a real project has
  the same transport ID. Agent persistence uses a disjoint actor setting object;
  malformed future archive shapes refuse writes, and unknown fields survive.
  Native hosting distinguishes application/project lifetimes, guards duplicate
  floating-window opens, and rejects late completion from a replaced runtime.
- Properties publishes applicable basic Content/Layer rows using Inspector
  descriptors, exact authored values, and existing control recipes. It computes
  mixed target type and shares selection storage between leases; target keys no
  longer serialize all selected IDs. Group key actions cover both axes and
  dispose their temporary lease. This does **not** complete every row/service.
- Settings project name/frame-rate/duration/background edits submit typed actor
  transactions. Profile values validate before publication; failed reads disable
  controls, failed writes keep acknowledged values and use approved error copy.
  Supported Switch/SecureField/minimal ColorWell/PushButton/FormGroup recipes
  replace placeholders. Secure fields carry a binding, never a published secret.
  Density is hidden until its real design/service backing exists.
- `ObservedRuntime` implements the media-runtime trait with project/document/
  revision fences and a Linux test double. The Mac worker probes images, decodes
  a first video frame, and reads an audio block before reporting Ready. Stale
  completions cannot cross a relink. Embedded migration proxies are classified
  as proxies using either provenance marker, never promoted to originals.
  Unsupported/unobserved media remains Unknown; filesystem metadata is not Ready.
- Delivery planning uses rational ticks, the engine's half-up frame-count rule,
  actual caption-layer count, strict work-area validation, authored still time,
  composition-relative preset scaling and exact custom dimensions. Options accept
  integral model f64 dimensions without truncating fractional inputs. Video uses
  the video color contract; image delivery uses the image contract.
- Mac capability reporting runs the planner and initializes the actual output
  sink in a temporary destination. Missing originals and disconnected sidecar
  delivery remain unavailable with reasons. Linux reports encoder unavailability;
  portable PMV availability independently checks every original file.
- Portable export writes a separate PMV4 with all original asset bytes/hashes,
  verifies a reopen in tests, refuses missing originals and existing destinations,
  and leaves the source untouched. Cancellation before publication creates no
  output; cancellation after atomic publication never deletes a completed file.
- Presets use main's **`renderPresets`** actor key, preserve unknown/malformed
  entries, archive same-name predecessors and append a fresh ID. Output names/
  destinations do not become reusable settings. No restore UI was added.
- Export hosting reserves one job across picker/startup/render/reap; duplicates
  cannot replace a live run. Retry retains the frozen publication/options/time/
  media/destination. Worker cleanup/reaping precedes terminal publication, and
  the task ledger receives queue/start/progress/terminal transitions. Reveal
  brings the running Export window forward. These Mac coordinator paths still
  require a Mac build and execution, including fault injection.
- Unsupported Agent submit/stop/answer/consent/provider effects refuse **before**
  consuming a draft or manufacturing a run. Local draft/thread/answer-selection
  operations remain model-owned. Open Settings is routed; active thread title is
  projected. Main's work-group planner is ported as a headless foundation:
  14 synthetic cases generated by executing pinned main plus duration cases
  compare exactly; a 60k-step plan keeps source ranges instead of copied payloads.
  It is not yet attached to the live native conversation list.
- Fixed-height virtual lists have validated spacers, bounded viewport math,
  stable scroll handles and range-change delivery. This is not variable-height
  Agent virtualization or offscreen VoiceOver navigation.

### Snapshot review and design follow-up reconciliation

The original **21 Inspector failures were checked individually**, not accepted
as a batch: removing only the new `metadata.symbol` entries made every original
case identical, including operation ordering, values, IDs, focus, edits and AX.
Those snapshots were regenerated in `6f6909d3b`. The later Inspector snapshot
changes are the approved D1 `To composition` tooltip detail. Other refreshed
snapshots reflect supported kit recipes, hidden Density and the explicit
Effects empty/error behavior below. They are synthetic wire/AX evidence, not
screenshots or visual approval. JSON render fixtures compare parsed objects
because enabling V8 changes serde_json object member order; array/draw order,
values and numeric comparisons remain exact.

Every review's Sol list was checked against current source. Current disposition:

| Review / items | Current code and remaining boundary |
| --- | --- |
| Kit2 1–2; Settings 8–10 | Merged secure-input, sampler, menu, wheel and fade adapters exist. Settings adopts supported field/button/card recipes. Persistent Keychain, native source-list/search, complete FormRow/emphasis and worker attachment remain. |
| Agent F1 | Supported Push placeholders replaced with system PushButton; unsupported actions stay disabled/refused. Native rendering/keyboard acceptance remains. |
| Agent F2–F4 | EmptyState/symbol and token primitives exist on merged inputs. App painting, bubble width/padding and native captures remain; do not add arbitrary geometry. |
| Agent F5 | Headless main work-group foundation added; live grouped disclosure is still open. |
| Agent F6 | Result kind/headings and real result/preview actions remain open. |
| Agent F7 | Active title and approved trigger label added; native picker and background-running count remain open. |
| Agent F8–F9 | Optical inset arithmetic, anchored NSPopover, native composer, variable-height list, pickers/previews/launcher remain open. |
| Effect Controls F0–F1 | Existing F0 retained; trailing Add Effect uses PushButton. Native centering/inset acceptance remains. |
| Effect Controls F2–F4 | Tokens and partial AX/symbol primitives already exist. Actual drag opacity/insertion geometry, mixed glyphs, per-kind row sizing and native custom-action completion remain. |
| Effect Controls F5 | Existing controller has initial-row/error/ack behavior; the live app's full parameter/action service route still needs verification. |
| Effects F1–F2 | Symbol/indent primitives exist; native NSSearchField/fill geometry and drag image remain. |
| Effects F3–F4 | Implemented empty/no-results/empty-scope as Tree siblings; absent Tree and disabled search for no installs. Approved search/apply errors announce once, retain prior results/focus, clear on specified changes and do not add recents. |
| Export F1–F2 | Supported PushButton/FormGroup recipes adopted. Full sheet form-row geometry and Mac native controls remain. |
| Export F3 | Existing menu detail/placeholder/AX primitives retained; app presentation/announcement/progress description checks remain. |
| Export F4 | Planning/preset failure feedback and frozen retry added. Destination-error repick, typed failures and complete transport-loss handling remain open. |
| Inspector F0–F4 | Existing symbol/heading/mixed-value primitives retained. Native multiline/expression/colour/font/pad/scrub services, heading geometry and complete mixed AX presentation remain. |
| Inspector F5 | D1 tooltip detail implemented. D2 rejected-edit/subscription-loss lines remain open in the live binding route. |
| Project F1–F3 | Existing search/card/ring/token primitives retained; native geometry/search/drag image/captures remain. |
| Project F4–F5 | Descendant search, Changed/progress/background fields, grid-column model and compact selection already exist; host must supply actual grid columns/progress/thumb facts. No substitute model was added. |
| Project F6 | Existing Unknown/transport-error presentation retained; observed native readiness is now supplied. Two-second Unknown/loading timing still needs host verification. |
| Project F7 | Native delete confirmation, drag image, marquee/autoscroll are still open. |
| Timeline F1–F6 | Merged symbol/badge/header/visibility/timecode/tab primitives and renderer retained. Full native geometry, parsing/routing, Scale hit mapping and optical insets remain acceptance work. |
| Timeline F7 | Native clip/key painting is merged. Graph editing/gestures, offscreen AX row navigation and physical captures remain open. |
| Viewer D1–D5 | Merged width-sample/intermediate-quality/unnamed-comp/play-source/stable-ID behavior retained; verify native pop-up width, gizmos, source player and focus across presentations on Mac. |
| Authority | Merged typed failure copy and setup factory exist; real controller/Keychain/env/provider service attachment remains. |
| Dialogs | Merged Answers and explicit Return/Escape behavior exist; main-thread Mac probe remains mandatory. |
| Documents / Editing / Recovery | Merged Banner / ViewerGizmo / EmptyState symbol adoption already exists. Actual viewer/window mounting, geometry, materials and announcements remain. |
| Library / Media | Merged Answers, stale-copy and native factories already exist. Window/sheet/input hosts and Mac geometry checks remain; the Library thumbnail-slot DESIGN-TODO from surf-follow is unchanged. |

`DESIGN-TODO(opus)`: per-kind structured/contributed and variable-font-axis
control details still need design review in the Properties presenter. No generic
replacement control or new appearance was invented. Earlier neutral placeholders
remain where their native services/recipes are missing. All output is provisional.

### Main-delta Agent audit (A rows)

| Row | Current lane result / still required |
| --- | --- |
| A01 | Read pinned shared/provider/Settings sources; native preset/discovery/tool-model filtering/address-key/Ollama-warning service is still missing. Do not advertise a connected compatible provider. |
| A02 | New Accounts/external-client spec merged. Multi-client installation/removal/copy receipts and config preservation need service/MCP owners; no external settings were changed. Claude Desktop still requires the owner's explicit opt-in. |
| A03 | Native classifier authority is not established; Project Auto must not be advertised as supported. |
| A04 | Exact headless work grouping added. Rich markdown, inert unsafe HTML/URLs, granted local playback/byte ranges, remote-image opt-in and live grouped UI remain. |
| A05 | Existing durable transcript retention stays; no unsafe visual HTML viewer was introduced. Safe reply publication/viewer, resize/pin/stream recovery remain. |
| A06 | Existing frozen MCP catalog not changed. Real external/headless tool lifecycle belongs to the MCP/service owners. |
| A07 | `project_memory` remains absent from the frozen native catalog; resolve in MCP/delta-data, retaining disabled data and non-Undo autosave semantics. |
| A08 | No foreign theme/fork/runtime authority was rewritten; extension/service owners must finish reversible appearance/rebase activation. |
| A09 | Existing realm/tree/sequence guards retained; application late-open/media completion checks added. Full provider/MCP recovery and authorization ordering remain. |
| A10 | Provider frame review/consent/error identity remains unverified; unavailable actions preserve drafts and existing edits. |

### Remaining blockers and ownership boundaries

The complete item 7/8 scope is **unfinished**. Agent provider execution needs a
real coordinator and compatible-provider/Keychain services; Settings account,
subscription, speech, extension activation/archive/runtime actions need their
actual services. Properties Type/structured/time/contributed/scene rows and the
native expression/font/colour/pad/scrub/menu services remain incomplete.

Export still needs destination-specific retry/repick, caption sidecar execution,
output preview/ETA/Finder completion UI, typed task errors, durable/inspectable
Background Tasks presentation and full failure/cancel fault tests. The current
`hasAudio` estimate includes video/precomp kinds; it must be replaced with the
engine's actual audible-voice plan before treating the audio row as authoritative.
Timeline media status is still its old Loading projection; only Project receives
the new observations. File-content changes without a document/catalog change need
native re-probing. Neither is hidden behind a passing readiness claim.

One **visible failing test outside owned paths** remains:
`pm-design --test spec::token_table_matches_spec_doc`. In
`apps/native/crates/design/tests/spec.rs`, `facts()` filters out
`colors.media_badge`, `metrics.kind_badge`, `metrics.thumbnail_min` and
`metrics.status_dot`, but the merged shared token specification already includes
all four. The design owner should remove that stale exclusion (restore full
coverage); this lane did not alter the assertion or regenerate the shared table.
Its separate suite has **8 passes and 1 failure**. No threshold, numeric tolerance,
blank-pixel fallback or skipped case was added to conceal it.

The app binary cannot compile/run on this Linux host: GPUI/Metal/AppKit/WebKit and
the macOS V8 sandbox artifact are required. Linux tests cover production portable
models and the surface modules through their Linux harness. The Mac-only
coordinator/controls/decoder code remains **uncompiled and unverified on Mac**.

### Decisions and selecting rules

| Decision | Selecting rule |
| --- | --- |
| Keep authored values exact, strict malformed ranges, rational timing, original-only closure, and observed readiness; do not substitute proxies or widen comparisons | N3 correctness/authored intent; N1 professional delivery |
| Archive replaced presets and keep opaque/future data; refuse malformed archives rather than reset; keep all old consumers with no proven replacement | N4 no user left behind; prune only after the last consumer |
| Give application Agent a disjoint durable namespace instead of reserving a possible user project ID | N4 data safety; N7 explicit inspectable scope |
| Apply only approved recipes/tokens/copy; leave incomplete controls neutral and list their design/service gates | N1/N2 and the owner's Opus design ownership |
| Refuse absent provider/native actions before model mutation; publish terminal export state only after cleanup | N3 truthful results; N7 inspectable lifecycle |
| Keep v4 API source/shape unchanged, separate V8 realms and native worker IO; no synchronous UI-to-JS waits | N6 compatibility; N8 native runtime boundary |
| Port main work grouping with a pinned executable oracle and source-index/range plans | N3 exact behavior; N7 bounded headless inspection |
| Merge the new main/3D specs, defer U-row implementation to the follow-on branch | Explicit owner routing on 2026-11-05; no new visual choices |

### Exact verification commands and counts

All commands run after activation at repository root. Native lockfile changes
only add direct dependencies already pinned in the workspace; `cargo update -p
pm-builtins` upgraded **zero** packages. No engine lockfile change is needed.

```sh
cargo test --locked --manifest-path apps/native/Cargo.toml \
  -p pm-builtins -p pm-panel-inspector -p pm-panel-media -p pm-panel-agentpanel \
  -p pm-panel-timeline -p pm-panel-viewerchrome -p pm-surface
# 662 passed, 0 failed, 0 ignored (pm-builtins: 26).
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-design
# 8 passed, 1 FAILED: token_table_matches_spec_doc (boundary above).
cargo clippy --locked --manifest-path apps/native/Cargo.toml \
  -p pm-builtins -p pm-panel-inspector -p pm-panel-media -p pm-panel-agentpanel \
  -p pm-panel-timeline -p pm-panel-viewerchrome -p pm-surface -p pm-design \
  --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/Cargo.toml \
  -p pm-builtins -p pm-panel-inspector -p pm-panel-media -p pm-panel-agentpanel \
  -p pm-panel-timeline -p pm-panel-viewerchrome -p pm-surface -p pm-native-app --check
cargo build --locked --manifest-path apps/native/Cargo.toml \
  -p pm-surface --examples -p pm-panel-inspector --example inspector_wire
cargo build --locked --manifest-path apps/native/crates/surface/standalone/Cargo.toml \
  --target-dir apps/native/target --examples
bun test --cwd apps/native/js/v4 panels app/native-number.test.tsx eval/panels.test.tsx
# 459 passed, 0 failed; 168 snapshots; 3488 assertions, no update flag.
bunx --cwd apps/native/js/v4 tsc --noEmit -p app/tsconfig.json
rustc --edition=2024 apps/native/crates/app/src/surfaces/followups_check.rs -o work/followups-check
work/followups-check prepare
CARGO_TARGET_DIR="$PWD/apps/native/target" work/followups-check test
# 334 passed, 0 failed; driver invokes locked Cargo on actual surface modules.
CARGO_TARGET_DIR="$PWD/apps/native/target" work/followups-check clippy
work/followups-check fmt
```

The changed TS/TSX files also pass Prettier. Every commit is staged-diff reviewed
and scanned by gitleaks. No new workspace crate, fixture from another lane, user
profile, credential, screenshot or build artifact was committed.

### Broader v4 verification — 2026-11-05

The complete `bun test --cwd apps/native/js/v4` run has **679 passes and 1
failure**, 178 matched snapshots and 4506 assertions across 52 files. The scoped
panel run above remains 459/459. The additional failure is deliberately retained:
`src/components/part2.test.tsx`, `Switch outside Settings is diagnosed without
changing the old Toggle checkbox` receives both `switchRequiresSettings` and
`switchOutsideSettings`, while its exact contract requires only the latter.

This is a renderer-merge overlap in **unowned**
`apps/native/crates/surface/src/kit/validation.rs`: the timeline input's added
ancestor scan sits alongside native-engine's existing, stricter top-level
`settings_host` guard. The surface owner should remove the obsolete ancestor-scan
block (from `if component == Some(Component::Switch) { let mut parent...` through
its closing brace) and retain the later `settings_host` check. Do not widen the
JS assertion to accept duplicate diagnostics, or remove the stricter guard. No
owned panel uses Switch outside a Settings root; their validation remains green.

Together with the design-token-table mismatch above, these are **two visible
shared-owner failures**, not skipped cases. The next integration needs these
precise surface/design reconciliations, plus the unfinished services already
listed, before a MERGE-READY finish signal is honest. This lane's final tracked
worktree is clean, and the native-engine base is an ancestor, but those facts do
not establish semantic or product readiness.

### Exact Mac on-screen acceptance checklist

Run the existing surface main-thread probe (`work/followups-check macos`) and a
locked app build/test first, with the pinned V8 sandbox artifacts and FFmpeg
setup from HANDOFF. **Do not count a refused/skipped Mac probe as a pass.** Then:

1. Launch with no project: open Agent and Settings from commands/menus. Verify
   no fake project/Project Settings section, independent draft/history, and
   keyboard focus. Open a real project, close/reopen, and repeat with a project
   named `builtin-application`; verify scope separation and no stale publications.
2. Open/close/reopen Agent, Settings and Export rapidly; issue duplicate opens
   during creation, switch documents with floating windows, and close the owner.
   Verify no Controller borrow crash, duplicate windows, or old-worker mutation.
   Agent remains a popup until the anchored NSPopover is implemented: do not mark
   its design accepted in this checkpoint.
3. Inspect all nine panels in light/dark/Increase Contrast and Reduce Motion/
   Reduce Transparency. Capture selected/empty/loading/error/focused states for
   Opus outside Git. Check numeric wells do not overlap; headers/timecode are not
   duplicated; Viewer is transparent over actual Metal pixels; required symbols,
   materials, tooltips and value columns match current specs.
4. Properties: text/shape/image/video/audio/mixed/locked selections; Content and
   Layer applicability, actual typed/colour/toggle edits and Undo; exact stopped
   and playing values; both-axis key action; selection/relink retirement mid-edit.
   Check font/expression/pad/menu/scrub errors are honest. D2 and unfinished row
   families remain release blockers, not accepted omissions.
5. Project: real image/video/audio ready/offline/corrupt/unsupported sources,
   migrated proxy-only media, missing originals, relink while a probe is running,
   and same-path file replacement. Inspect badges/AX/no false Ready, thumb/grid
   geometry, descendant search, row navigation, delete alert, drag/marquee and
   cloud failure. No decoder result may come from metadata/proxy presence alone.
6. Effects/Effect Controls: installed/missing definitions, no selection/no
   installs/no results/empty scope, rejected search/apply, next-change clearing,
   no failed recents, focus retention, header and trailing Add Effect, parameter
   writes/Undo, reordering/insertion line, announcements and custom AX actions.
7. Settings: system Switch versus Checkbox roles; native PushButton default/
   destructive behavior; minimal color well; secure entry/IME with no value in
   surface capture; acknowledged profile/project writes and rejection; external
   change refresh; Density absent; extension filter/list/loading and service
   unavailability. Real account/provider/Keychain/client actions require their
   service integration before this check can close.
8. Export every supported format using real codec/planner probes; test NTSC,
   absent/invalid work area, custom/preset dimensions, still at nonzero playhead,
   caption count, alpha/audio/motion blur, unsupported settings and unavailable
   originals. Compare summary with decoded output frame count/dimensions/color/
   audio. Sidecar must stay unavailable until delivered; check silent video/
   nested/muted audio against the actual audible-voice plan.
9. Portable PMV: reopen a separate export with every original byte present,
   including unused assets; source/dirty/history unchanged, no proxy substitution
   and no overwrite. Exercise preset replacement/restart and opaque old entries.
10. Export lifecycle: cancel destination picker, duplicate begin, cancel before
    child creation, cancel during render, child crash, encoder failure, disk full,
    missing/unwritable destination, close/reopen, switch project and quit. Verify
    reservation release, no orphan children, cleanup before terminal UI, retained
    output, frozen retry and task transitions. Destination repick and remaining
    completion/task UI are still implementation blockers.
11. Agent: refused provider actions preserve exact draft and selection, Open
    Settings works, active title is spoken, and local thread operations persist.
    After real provider integration, run submit/steer/stop/question/consent/retry/
    disconnect/late-publication cases plus A01–A10; verify inert rich text/media
    grants and timer continuity. The headless grouping fixture is not this pass.
12. 60k lists: wheel/trackpad momentum, resize, paging and re-publication retain
    focus/scroll by ID; no blank tail or huge tree expansion; VoiceOver can reach
    offscreen rows. Explicitly test the still-unimplemented variable-height Agent
    and offscreen focus paths before acceptance. Native text controls keep IME,
    Unicode/UTF-16 selection, Return/Shift-Return/Escape and system paste ownership.
13. Timeline/Viewer and merged surface families: actual clip/key/ruler/graph
    rendering and pointer gestures; source/transport controls, zoom/quality/width,
    overlays/gizmos and focus across layouts. Run authority/dialog/library/media/
    recovery default/Escape/stale-target and material checks from their reviews.
    Leave U-row tab-stack/maximize/per-axis/3D work to the follow-on lane.
