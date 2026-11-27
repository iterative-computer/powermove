# Viewer chrome model

Wave 4 head start, based on `feat/native-engine` at `ff42d751f` and the shipped
`v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`). Package:
`apps/native/crates/panel-viewerchrome` (`pm-panel-viewerchrome`). **Model only;
no visual output, no claim of application cutover or complete viewer parity.**
All future visual output remains provisional until Opus design review.

## Kernel and host boundary

`Controller` reads immutable kernel publications. `ViewModel` contains current
composition dimensions/timing, ID selection, zoom/pan/fit, quality, controls and
explicit overlay geometry. It does not own a document store or media clock.
`ViewerQueries` wraps the kernel query mirror and checks document lifetime and
revision. Missing fonts/models/host answers stay typed errors, never empty hits.

`plan` and `drive` return inspectable `Plan`s. View changes merge into existing
`UiState` through a state-revision-guarded `StateTransaction`; unknown extension,
transport and perf fields survive. They neither dirty the document nor add Undo.
Work-area changes are typed `CompCommand`s inside revision-guarded kernel
`Call::Transact`, including normal kernel Undo/Redo, folding and durability notices.
Selection uses kernel `SelectionCall` and stable IDs. Add retains key selection.
`Submission` exposes the original pending acknowledgement; hosts must preserve
kernel `Reply::Throw`, folding and durability notices rather than infer success
from enqueueing.

Playback start/pause and seeks during playback enqueue `HostAction::Transport`.
The host must enforce media readiness and coordinate audio/decoder/clock acceptance
before publishing observed facts. Paused seeks publish frame-aligned time; the
engine consumer still owns preparing/presenting those pixels. Quality changes
preserve playback time, playing/rate and opaque perf data. `preview_size` describes
the full-frame fallback; cropped viewports/caches are engine-service work.

`ViewerHost::enqueue` is the platform seam; the Linux `InspectHost` only records
requests. Source state/effects and 3D toolbar transitions are executable pure
models. They do not claim that a decoder, camera navigator or DOM menu ran.
Source effects must run on a worker, with source epoch plus document identity;
late observations from a previous load of the same asset are rejected. Failed
source resolution retains the currently open preview. Images have no transport;
audio/video use source seconds, independent of project FPS.

`input::resolve_key` describes shipped built-in candidates. Shell registry
priority, extension overrides, field arbitration and false/async-decline remain
shell-owned. After that arbitration, use `command_action` to get the native
model action or an exact delegated command ID. Space can expose temporary Hand
without consuming the global Play command. Blur clears temporary Hand. Native
scroll events are classified as pan, zoom-pixels or magnify by the platform host;
Chromium `wheelDeltaY` heuristics are not a native API.

## Behaviour inventory

All references below are `v1.2.0:apps/desktop/src/`. “Done” means executable,
headlessly tested model behaviour. “Host” means the semantic model/request is
done and real execution is still required. “Not done” explicitly excludes a
parity claim; the reference implementation remains a consumer and is retained.

| Behaviour | Immutable source (line) | Native model / status | Coverage |
|---|---|---|---|
| Fit, 5/12.5/25/50/100/200/400/800% and custom choice | `extensions/viewer/viewer.ts:855`, `:923` | `Navigation::choice`, `ZOOM_PRESETS`; done | navigation presets/custom/resize |
| Explicit zoom clamps 5–800%; Fit display can reach 2%; entering manual resets Fit pan | `extensions/viewer/viewer.ts:920`, `:1000` | `set_zoom`, `shown`; done | clamp/nonfinite/Fit cases |
| Return to composition; offscreen-only recovery suppressed during source preview | `extensions/viewer/viewer.ts:1006`, `:1011` | `fit`, `out_of_view`; done | recovery and boundary cases |
| Composition point remains under pointer through magnification | `extensions/viewer/viewer.ts:218`, `:1063` | `zoom_at`, fractional coordinates; done | reversible zoom + tag oracle |
| Continuous pan; Shift maps vertical scroll to horizontal; wheel/pinch zoom bounded per event | `extensions/viewer/viewer.ts:233`, `:1566` | `Scroll`; done; host classifies native events | pan/remap/bounded zoom |
| Middle/Hand/Space drag pans; live Shift doubles speed | `extensions/viewer/viewer.ts:1766`, `:1779`, `:1832` | `PanDrag`; done; host routes button/tool | from-start pan, Shift, temporary Hand |
| Zoom-tool click ×1.25, Option click ×0.8; rectangle zoom; Option release overrides rectangle | `extensions/viewer/viewer.ts:1886` | `zoom_release`; done | click/Option/marquee/ephemeral cancellation |
| Preview Auto/Full/Half/Quarter; Auto resets scale to 1 and enables adaptive quality | `extensions/viewer/viewer.ts:878` | `Quality`, merge preserves perf fields; done | all choices, opaque fields, time/rate unchanged |
| Full-frame density-aware Auto and explicit resolution independent of zoom | `extensions/viewer/viewer.ts:28`, `:975` | `preview_size`; done; crop/render host pending | resolution vectors + tag oracle |
| Toggle boxes/handles without changing content visibility | `renderer/src/legacy/ui/shortcuts.ts:845`; `extensions/keymap-default/keymap.ts:66` | `ToggleControls`; done | history/dirty unchanged, Cmd/Ctrl+Shift+H |
| Zoom shortcuts . , / Shift+/ and Cmd/Ctrl+=, Shift++, -, 0, 1 | `extensions/keymap-default/keymap.ts:63`, `:147` | exact command IDs/modifiers; done | every alias, field/repeat/extra modifier rejection |
| Space play, Home/End, work bounds, frame steps, Shift ten frames, Cmd arrows | `extensions/keymap-default/keymap.ts:109`; `renderer/src/legacy/core/engine.ts:280` | paused seeks/steps done; playback host | keymap + project-frame/pause-floor tests |
| J/K visible events; Shift+J/K and Ctrl arrows real adjacent keys; I/O layer edges | `extensions/keymap-default/keymap.ts:125`, `:133` | exact service command delegation; host | IDs/arguments, repeat refusal; timeline executes |
| B/N and Shift+B/N work area; reset | `renderer/src/legacy/ui/shortcuts.ts:381`; `extensions/timeline/timeline.ts:4418` | guarded kernel transaction; done | one-frame span, Undo/Redo/reset, stale rejection |
| Field, repeat and modifier admission; Escape active drag | `extensions/keymap-default/keymap.ts:10`; `extensions/viewer/viewer.ts:1594` | candidate routing/cancel signal; done; shell owns registry | all routes and precancel |
| Source show/toggle by asset ID, same asset reuse, replace/clear, pause composition | `extensions/viewer/source-preview.ts:172`, `:181`, `:217` | `SourceState::plan`, load/release effects; host | same ID/replacement/missing/offline/image cases |
| Source picture/play button toggles; restart at end within 0.01 s | `extensions/viewer/source-preview.ts:222`, `:248` | pure playback intents; host | toggle/pause/restart cases |
| Source slider pointer capture and continuous scrub | `extensions/viewer/source-preview.ts:238`, `:258` | ratio seek done; host owns pointer capture | ratio bounds/no project-frame quantisation |
| Source slider Left/Right ±5 seconds, Home/End; Escape/close dismiss | `extensions/viewer/source-preview.ts:252`, `:270` | pure commands + key routing; host | every key and close effects |
| Context menu raw hit includes locked layers; empty menu gathers viewer contributions | `extensions/viewer/viewer.ts:1531` | kernel pick + `ContextMenu` request; host | locked ID routing/selection unchanged; dynamic menu host |
| Click/Shift selection and multi-select by stable ID | `extensions/viewer/viewer.ts:1851` | replace/add/toggle plans; done; no implicit move | selection tests including locked IDs and duplicates |
| Cmd/Ctrl deep pick, group entry, double-click drills one selected group | `extensions/viewer/viewer.ts:656`, `:668` | `pick_target`; done | nested groups/deep/sibling/drill |
| Empty click clears; marquee fully encloses active pickable world bounds; Shift symmetric difference | `extensions/viewer/viewer.ts:1922` | `Marquee`; done; tools evaluator supplies stamped world bounds | inactive/unpickable/partial enclosure/cancel |
| 3D Move/Rotate/Scale returns navigation to Select; world/local, camera/editor, navigation/framing | `extensions/viewer/GizmoToolbar.svelte:20` | `SceneState`, stable IDs; host | all mode families/framing keeps navigation |
| Agent inspection and every chrome action driven by ID | no shipped Rust equivalent; N7 | `inspect`, `ELEMENT_IDS`, `drive`; done; MCP wiring pending | every ID resolves, invalid args fail, identity strings |
| User guides/safe-area/grid/rulers | no shipped toggle found; W045 B115 | explicit geometry + guides/safe-area visibility only; neutral additions | empty defaults/validation/unknown-field preservation |
| Renderer last-frame retention, viewport crop/reuse/refinement, same live surface across reload | `extensions/viewer/viewer.ts:747`, `:898`; `extensions/viewer/index.ts:56` | **Not done here**; engine-service/shell integration | macOS render gates remain |
| Canvas move/resize/rotate/anchor and snapping; pen/shape/text tools; full 3D navigation | `extensions/viewer/viewer.ts:1834`, `:1957`, `:2111`, `:2290`; `path-editing.ts:59`; `text-editor.ts:86` | **Not done here**; viewer-tools consumers, not chrome | retain reference; need a tools-model lane |
| Text double-click/Enter, precomp/3D double-click and effects/composition drops | `extensions/viewer/index.ts:38`; `extensions/viewer/viewer.ts:1623`, `:1650`, `:1679` | Enter delegates exact command; other tool/drop interactions **not done** | command routing covered; owning tools/DnD lane required |

Context contributions are deliberately not frozen into new menu copy. The tag
has dynamic extension menu callbacks; no Fit/Settings menu rows are invented.
This crate is **not** a v1–3 `services.get('viewer')` facade: C-MATRIX's connected
synchronous editor closure stays in its V8 compatibility realm until complete
identity/callback traces prove a facade. No Node/Chromium dependency is added.

## Agent and scale contract

`inspect()` returns semantic data and 28 element IDs, with 64-bit revision and
document identity encoded as decimal strings. `drive` validates IDs/arguments;
selection/context/source target existing IDs. A headless host can consume the
same plans as a future visual host. It must not report MCP success merely because
`InspectHost` accepted an item. No MCP catalog or public schema is changed.

There are no viewer row lists to virtualize. `selection_page` returns at most
1,024 IDs and safely handles overflowing offsets/limits. Model inspection does
not traverse the layer list; selection planning validates using sets. A real
60,000-layer/selected-ID test exercises paging and submission. Kernel's existing
`tx::select` validates each selected ID by scanning its layer list, so select-all
still has a quadratic kernel cost (about 22 s in this unoptimized Linux fixture).
This is **not** a 60k interaction-performance pass. Kernel selection validation
needs a set-based lookup in its owning lane; ordinary chrome inspection is bounded
apart from cloning the selected-ID snapshot and explicit overlay collections.

## Decisions and selecting rules

| Decision | Rule |
|---|---|
| One kernel writer; async pending acknowledgements, no second document/history authority | kernel architecture; N3/N8 |
| View state uses guarded UI transactions; authored work-area edits use typed document transactions | N3 exact timing; N4 no partial writes; N7 inspectable plans |
| Keep fractional navigation and project-frame snapping; pause floors to the visible frame | N3 authored behaviour; tag engine.ts:280/334 |
| Publish playback only after real host acceptance; conditional picks remain errors | N1/N3 correctness; N7 honest inspection |
| Failed/unavailable source replacement keeps current preview; observations are epoch fenced | N2 strictly better; N4 preserves working state |
| Preserve opaque fields and all old providers; delete nothing | N4/N6 and prune-only-after-last-consumer |
| Expose native scroll intent instead of emulating Chromium wheel metadata | N3 plus the quirk policy: preserve authored navigation, not browser detection |
| No safe-area ratios, guide editing defaults, UI geometry/copy/fade values chosen | Opus ownership; N1/N3; neutral semantic placeholders |
| Keep exact delegated command IDs and dynamic contributions | N6 callback/override contract; C-MATRIX E closure |
| Model-only agent IDs do not claim a frozen MCP body or v1–3 live-object facade | N6/N7 honest contract boundaries |

## DESIGN-TODO(opus)

- Approve native fit inset and zoom-marquee movement/minimum threshold. The model
  requires host-supplied values; tests use the tagged 8/3 values as fixtures only.
- Specify safe-area presets/ratios, guide-editing interactions and persistence,
  grid/ruler behaviour, overlay ordering/presentation. Defaults are empty/hidden.
- Specify Focus/fullscreen overlay reveal/hide timing and focus transitions. No
  fade timers, colours, metrics, typography, layout or new display copy exist here.
  Existing design language §10 and pm-design tokens remain authoritative.

## Integration outside this lane

1. Add the generated package entry below to `apps/native/Cargo.lock` (owned by
   the integrator). The workspace's `crates/*` glob already discovers this crate;
   no root Cargo.toml change is needed. Only this package entry changes resolution.

   ```toml
   [[package]]
   name = "pm-panel-viewerchrome"
   version = "0.1.0"
   dependencies = [
    "pm-kernel",
    "pm-model",
   ]
   ```

2. Add kernel actor consumption-time guards for document identity + expected
   document/state revision + expected selection for state/selection/document
   plans. `Controller::submit` checks the full stamp before enqueueing and the
   actor separately guards state/document revisions; the current actor API cannot
   atomically fence a close/reopen or selection-only change occurring between that
   check and consumption. Do not treat this as a solved multi-client race. Hosts
   must serialize lifecycle/selection dispatch until that actor operation exists.
3. Bind source effects/media observations, engine/audio transport, dynamic menus,
   3D services, pointer capture and native gesture classification through
   `ViewerHost`. Check stamps/epochs again at consumption/presentation. Adapt
   controller plans to v4/surface/MCP after API freeze; no UI or MCP files changed.
4. Retain viewer tools and compatibility editions until complete replacement,
   callback/identity, authored timing and renderer gates pass. Improve the kernel
   select-all guard for 60k+ selections. These paths are not owned by this lane.

## Linux validation

Tests cover pure behaviour and the real kernel actor with synthetic projects,
including one-frame work bounds, Undo/Redo, foreign state, stale queued plans,
reopening IDs, source epochs, group selection and 60k selections. The eight checked-in
navigation vectors were generated by executing actual tag functions using
`tests/oracle.mjs`; Node is required only to regenerate those fixtures. Rust tests
read the committed fixture without Node.

Verification uses pinned Rust 1.96.1 in a disposable worktree with the single
lockfile package entry added there, respecting this lane's path ownership:

```sh
cd apps/native
cargo test --locked -p pm-panel-viewerchrome -p pm-kernel
cargo fmt -p pm-panel-viewerchrome -- --check
cargo clippy --locked -p pm-panel-viewerchrome --all-targets --no-deps -- -D warnings
```

The unmodified integration checkout's `--locked` build requires the lockfile entry
above. Final result: **53 panel-model tests passed; 273 existing kernel tests passed;
1 optional real-profile kernel test ignored; 0 failures**. Package formatting and
strict all-target Clippy pass with no warnings in this crate. The final panel run
includes the cancellation guard fix; the kernel was unchanged after its combined
passing run. The reference-reader suite required fetching both v1.2.0 and v1.3.0;
the first attempt failed on the absent v1.3.0 tag, then the complete rerun passed.

macOS still must verify native wheel/magnify/button/focus/IME routing, source decode
and seek/playback acceptance, real dynamic extension menus/3D services, audio
continuity through quality changes, geometry/overlay alignment at display scale,
EDR and retained-frame/crop behaviour, lifecycle/selection fencing in the app,
MCP/v4 adapters and the Opus visual review. Linux tests are not those proofs.
