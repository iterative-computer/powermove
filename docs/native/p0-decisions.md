# P0: decisions and open items

Status 2026-11-01. Plan v6 is the overall implementation plan; not every
decision is Jude's. Technical calls within the plan's rules are made by the
orchestrator and recorded here; product calls go to the owners (Chike, Jude).
Each item names its evidence. Nothing here authorizes dropping a v1–3 member
(N4) or changing the frozen reference.

## Gates

| Item | State | Evidence |
|---|---|---|
| S1 V8 | **Works; gate open.** rusty_v8 152.2.0 (V8 15.2.124.1) with the V8 sandbox + pointer compression built from source on GitHub Actions macOS runners (owner decision: CI), verified artifacts reused locally; Store helper 9.21 MB, stop/recreate 1.39 ms, snapshot restore p95 4.65 ms, signed TurboFan JIT OK. Open: reconciler timing, hard external-memory limits, attribution, cluster integration. | `s1-v8.md` |
| S2 GPUI-CE | **Open.** GPU tests pass (P-DEVICE, P-HEADLESS, P-TEXTURE). Unlocked rerun: dark glass toolbar PASS, Reduce Transparency fallback PASS, field menu spring 0.32/0.15 PASS, cross-slot drags limited PASS; **accessibility FAIL** (slot children absent from the AX tree, so P-A11Y is required); 120 Hz cadence p95 8.33 ms but CPU p95 6.88 ms vs 4.17 ms headroom (**FAIL**); resize ≤1-frame unproven; IME needs a manual check; one display, so no display-move test. Visual defects: light titlebar over dark content, overlay label clipped. | `s2-gpui-ce.md` |
| S3 Viewer (EDR) | **Open.** RGBA16F below-slot EDR layer works (headroom 1.2×→2.67×, linear values preserved to 8.0); engine-drawn safe-area overlay 0 px offset; playhead/timecode as native layers with 0 GPUI repaints in playback; viewer = export ΔE p99 0.03 (SDR) / 0.06 (PQ input). **Fails:** 120 Hz soaks show 6–11 scanout gaps per 60 s; scrub input→present p95 45.7 ms vs vsync+1. Open: PQ/BT.2100 output, animated gizmos, display moves. | `s3-viewer.md` |
| S4 GLSL | **Draw regression passes; gate open.** 661 cases / 1,605 variants by stage: 516 draw natively, 13 cases need the WebGL route, zero naga failures (so no SPIRV-Cross route), zero lost draws vs the previous writer; killable compiler processes. Open: production Metal compile still host-side, no historical WebGL captures, one self-unstable case. | `s4-glsl.md` |
| S5 Compat | **Open, with failures** (below). | `s5-compat.md` |
| S6 Migration readers | **Readers work; cutover proofs open.** Snapshot-only IndexedDB/LevelDB/V8/Blob and localStorage readers; resumable BLAKE3 media move at ~284 MiB/s, 5 MiB RSS. Real-profile proof on this Mac: 593 records (442 media, 151 posters) describing ~15.9 GB, read without Electron. Open: crashed-Chromium recovery, OS-denied blobs, rollback closure, signed Keychain/updater proofs, agent-client replay. Plan: two-DR Keychain ACL, TR-gated Squirrel, then Sparkle 2 with new EdDSA keys. | `s6-migration.md` |
| Inventories | Done: W1, C-MATRIX, D-STATE, T1, OS base. | `w1-*.md`, `c-matrix.*`, `d-state.md`, `t1-themes.md`, `os-base.md` |

## S5 failures

First measured with the Mac locked, then rerun unlocked and confirmed foreground (2026-11-01). **The unlocked rerun did not rescue the budgets**: HTTP sync p95 0.78 ms (fails ≤ 0.5 ms; the scheme handler passes at 0.10 ms), 1080p pixels 92/72 ms, continuations p95 287 ms, background throughput 28–41% (worse than locked), hang kill via the named SPI still fails while the reset-state SPI recovers in 99 ms. The numbers below are the locked run.

1. **Pixels page ↔ app:** 1080p download/decode p95 135 ms, binary upload 51 ms, text upload 171 ms against the ≤ 10 ms sync-pixel budget. Recommendation in the spike: keep pixels in the engine and pass ids; the sync `renderFrameTo` canvas fill is the remaining cost.
2. **Render continuations:** work without deadlock (inline override, nested capture, real canvas, JPEG), but p95 384 ms on the synthetic path.
3. **Background activity:** with the visibility/throttling SPI, a page-driven job kept 74% (app hidden) and 69% (windowless) of foreground throughput, short of the ≥ 95% rule. Offscreen-window-only stalled.
4. **Hang kill:** `_killWebContentProcess` and WebKit's own termination did not end a real infinite loop within 20 s; a different reset-state SPI restarted the page in 106 ms (old process confirmed gone).
5. **Fallback** (trusted realm in V8 + remote DOM): evaluated in S5c and adopted (orchestrator decision 5).

Decision needed (now firmer after the unlocked rerun): choose the scheme handler for page→app sync calls, and either accept S5 as "channels viable, pixel/continuation/background budgets need the engine-id design and an unlocked rerun", or trigger the V8 + remote-DOM fallback evaluation (blocked on S1).

## Orchestrator decisions (2026-11-01)

1. **Reference build:** tag `v1.2.0` (the plan's definition of "today"). The async render siblings added after the tag are supported too (a superset); traces come from v1.2.0.
2. **Authored vs fixed:** decided case by case under §4's quirk policy; each decision is recorded beside its fixture/trace.
3. **S5 direction:** page→app sync uses the scheme handler (0.10 ms p95); pixels stay in the engine and are referenced by id; a focused spike on continuation latency and background activity follows; the V8 + remote-DOM fallback is evaluated once S1 can run.
5. **Compat architecture (after S5c, 2026-11-01):** adopt the plan's documented fallback. The trusted v1–3 realm runs in **V8**: extension logic, `PM`, synchronous services, raster overrides and sync pixel APIs, using the recorder canvas rasterised natively. **WebKit** only renders DOM panels through a remote-DOM bridge. Rule: §5 budgets plus N4/N6 (synchronous behaviour unchanged). Measured: V8 sync pixels 1.3–3.9 ms vs WebKit 10–31 ms; cold continuation 3.1 vs 14.3 ms; background throughput ~116% vs 24–32%; DOM key→render 36 ms via the bridge vs 21 ms in WebKit (to optimise). The synchronous service cluster stays in one realm. Open: JPEG snapshot of noisy frames (21 ms), full DOM/canvas semantics, pm-vector/pm-text raster integration, the 39-extension memory row.
4. **Pro features missing at v1.2.0** (J/K/L, ripple, linked A/V, slide/roll, three-point editing) are cutover work, as §1 already states.

## Owner decisions (Chike, 2026-11-01)

- **Eval specs:** anonymised behaviour specs of Chike's panels may be used in the v4 eval (no names, ids or code in tracked files).
- **Store-wide counts:** pull published Store extension sources read-only through the Store tools; record usage counts only.
- **S1:** runs on a cloud/CI macOS runner (this Mac lacks the disk).
- **Design calls (language.md §17):** all five approved: left-aligned values, neutral timecode, neutral list selection, workspace switcher in Window ▸ Workspace (+ optional toolbar item), 32 pt panel headers.
- **Panel product calls:** timeline layer list 266 pt default; agent opens as a popover and can be pinned; "Effects & Presets" renamed Effects until presets exist; web-animation export removed (see design/panels/README.md).
- **v4 API frozen (orchestrator, 2026-11-01):** after the integration lane, the headless eval passes 40/40, including all 10 owner-panel specs. The v4 contract on feat/native-engine is frozen for built-in panel work. Changes from now on are additive or need a recorded break. Production integration (real kernel binding, macOS UI acceptance of the owner panels) continues in parallel and does not reopen the API shape.
- **Edit/scrub latency endpoint (P1 §5 rows):** split into two checks. (1) Powermove's own share, input → frame ready for presentation (kernel + service + render, ending at GPU completion), p95 ≤ 9.333 ms at 120 Hz. (2) End to end, input → on screen (`presentedTime`), native must beat Electron measured the same way on the same Mac. Reason: macOS composition alone adds ~20–24 ms for any windowed `CAMetalLayer` (perf-gate-p1.md, "Latency pass"), and Electron's reference numbers stopped at GPU completion + rAF. Approved by Chike.

## Plan adjustments (owner, Chike, 2026-11-01)

Plan v6 was written around one person's machine and workload. Where a rule only reflected that, it now follows the hardware and people actually doing the work:

- **Timing machines.** The 120 Hz XDR reference machine is any Apple-silicon MacBook Pro with a 120 Hz XDR display. The build Mac (base M5, 16 GB) now fills that role; there's no requirement to run on one person's M4 Max. The second required machine is still "the cohort's most common base M-series at 60 Hz", because that's about users, not about who builds. Until one is available, 60 Hz rows run on the build Mac with the display set to 60 Hz and are labelled *stand-in (faster than typical)*.
- **Capacity.** The rule was 6–8 local lanes on 128 GB. It is now at most 2 local lanes and one compile at a time (`flock /tmp/pm-build.lock`), with ≥ 8 GB disk kept free. CPU-side and JS work runs in cloud lanes.
- **E-track (Rust engine finish).** The E-track becomes ordinary lanes run by the orchestrator. It starts from the open `feat/ne-*` branches (`ne-bigrender`, `ne-l4comp`, `ne-l5stills`, `ne-phase4a`, `ne-svg`). Their author keeps review rights, but Gate E no longer waits on one person.
- **No single-person gates.** Nothing waits on Jude. His plan roles move: E-track → orchestrator lanes; the 2-round design review per surface → Opus design review plus the owner (Chike); release/`main`/TR approval → Chike. The latency endpoint decision stands on Chike's approval.
- **Design and models.** Opus owns design (spec and review). Implementation goes to GPT-6.1 Sol by default, to conserve Claude usage.

## Questions from the inventories (original wording, kept for traceability)

1. **Reference build.** §2 says `renderFrameToAsync`, `snapshotAsync` and `rasterAsync` ship to every apiVersion; at tag `v1.2.0` they don't exist in `RenderAPI`/`makeRender` (only `PM.Export.snapshotAsync`). Which build is the frozen reference for traces? (C-MATRIX Q1)
2. **Store-wide counts: answered (2026-11-01).** Anonymous public Store reads covered all 4 current catalog extensions; C-MATRIX records usage/occurrence counts and risk classes, with 0 direct DOM/canvas/WebGL, raster-override, sync-pixel, live-service or `PM.proj`-write users and 4 GLSL-effect users. (C-MATRIX Q2)
3. **Authored vs fixed.** Where today's code and §3.2.1 differ (async decline replay, literal-false/pre-cancel policy, partial workspace mutations, Promise-returning mutation callbacks), which are authored contract and which are bugs to fix? (C-MATRIX Q3)
4. **Pro features missing at v1.2.0** (W1): J/K/L shuttle, ripple delete/trim, persistent linked A/V, slide/roll, source-monitor three-point editing. §1 makes them cutover items "if W1 shows any are missing today". Confirm they are new cutover work.
5. **Eval specs** (§12, open): may anonymised behaviour specs of Chike's panels be used in the v4 eval?

## Recorded owner decisions (Chike, 2026-11-01)

- This Mac runs P0 except S1. The app grows from `feat/na-foundation` (`apps/native`).
- Web-animation export is removed (2026-10-29), not a cutover requirement.
- Verified lanes are pushed as `feat/na-*` and merged `--no-ff` into `feat/native-engine` as they finish.

## Owner decisions (Chike, 2026-11-04)

- **Behaviour reference moves to latest `main`.** The 1:1 parity target is `origin/main` at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, replacing `v1.2.0`. [main-delta.md](main-delta.md) is the work list. If `main` moves, re-run that audit from the new tip. v1.2.0 stays a required input: every project and extension from it must still work.
- **The new 3D system ships with the rewrite.** Port it to Rust from `feat/na-3d-source` (Electron `core/scene3d` and `core/studio`, plus `engine/crates/pm-studio`). Old 3D projects upgrade automatically, with no compatibility mode and no Blender.
- **No restore UI** for archived threads, takes and presets. Archives are kept.
- **Ask first** before adding Powermove to the Claude desktop app's settings. No silent registration.
- **First-run animation** plays inside the Welcome window.
- **Going back:** there is no reverse migration. The old app must refuse a native file it can't read, safely and clearly, instead of opening it wrongly.


## Core lane decisions (2026-11-07)

- **Stale locks:** holding the flock proves no local owner, so a dead pid on a local volume is taken at once; a record younger than 30 s blocks only while its pid runs or on a shared volume; any record whose heartbeat stopped 30 s ago is stale.
- **External files:** a `.pmv` opened from disk is edited in place under its `.pmvlock`; its library entry is the file under its own name. Only copies (locked, cloud, read-only, foreign library) get " — Native".
- **Journals:** a project that closes saved and durable retires its journal. Quit compacts only library-owned work; an external file's unsaved work stays in its journal for the next open to ask about (only the restore build's `CompactAll` writes files). A crashed session is archived once a recovery copy holds it.
- **Recents of a file:** a library entry that stands for an external file opens through the file route (same journal base, lock and recovery as opening the file). A copy never records the original file as its own.
- **Relaunch:** a restored window reopens its project when its journal holds unsaved work, or when the system keeps documents across quits.
- **Accessibility before activation:** with forced accessibility, the newest tree built before an AX client connects is handed to the adapter on activation (vendored GPUI), so idle or occluded windows aren't empty.
- **Projects from before compositions:** a missing `compId` is filled at open, derived from the project id so every open of an unsaved file has one journal base; nested comps are left as stored.
- **Shader cache:** translated built-in shaders live in `<profile>/shader-cache` (keys carry pipeline revision, translator versions and device).

## v4 API additions

- **`onPointerEnter` / `onPointerLeave` (2026-11-07, additive):** typed on every intrinsic element; the host already delivered `pointerenter`/`pointerleave` to listeners. Timeline rows use them to show their switches on hover.
- **`EventPlan.semantic` (2026-11-08, additive, host↔renderer wire):** optional; the host sets it to the target's `id` when it planned an activation on an identified element, and the renderer then accepts the event even if a render restyled that element first (a hovered row stays clickable).

- **`PopUpButton` `display` (2026-11-08, additive kit prop → `metadata.pmKit.display`):** the button's title when it differs from its selected row; the native `NSPopUpButton` shows it via its cell's own menu item (the viewer's zoom reads `Fit (54 %)` while its row is `Fit`).

- **`metadata.submitOnReturn` on `textarea` (2026-11-08, additive):** the host draws a native composer (`TextKind::Composer`, an NSTextView: IME, spelling, dictation, emoji and shifted punctuation are AppKit's). Return sends `input` with the text, then `keydown` Enter; Shift- or Option-Return inserts a line. The field clears after a send once the model empties it, unless the person typed on. The Agent panel's composer uses it.

- **`activate({ themes })` / `env.themes` (2026-11-08, additive):** `themes.register({ id, title, tokens })` offers a whole-window theme as palette "Theme: <title>"; the app applies its token patch through the shell's theme pipeline (`pm_shell::theme`), restyling chrome and every panel in every window; it follows reloads and leaves with its extension. Tokens only (no surface rules yet); the choice lasts until quit. `themes.activate(id | null)` (additive) shows one now, as v1–3's `theme.activate`: the latest activation in the window wins (a repeat counts); `null` puts back what that extension's theme replaced (the active v1–3 theme, or the built-in look).

- **`activate({ menus })` / `env.menus` (2026-11-08, additive):** `menus.contribute(location, ctx => items)` with v1–3's `MenuContribution` shape and 100 ms answer limit; the app gathers v4 items off the main thread, then v1–3's, into panel tab (`panel:context`), layer (`layer:context`, timeline rows, clips and viewer layers), timeline (`timeline:context`) and empty-canvas viewer (`viewer:context`, with the shipped New submenu) menus. `titlebar:right` isn't gathered yet (no extension toolbar slot).

- **`activate({ overlays })` / `env.overlays` (2026-11-08, additive):** `overlays.viewer(Component)` renders `<Component frame />` over the composition, below the transform gizmos, as v1–3 extensions drew on the viewer's overlay canvas. `frame` is the composition's rectangle in overlay points and follows pan and zoom; display only (pointer input passes to the viewer). It publishes as `<panel>@viewer` and leaves with its extension.

- **`activate({ timeline })` / `env.timeline` (2026-11-08, additive):** `timeline.chrome(patch)` writes v1–3's `workspace.chrome.timeline` fields (row height, gutter width, ruler height, clip radius, key size, surface order) with v1's `sanitizeInterfaceEdit` ranges, as v1–3 extensions did through `workspace.mutate`. Layer numbers, type badges and toolbar density throw until the native timeline draws them. Edits go to one writer thread per window that reads the kernel's workspace, changes only the named fields and writes it back, so none is lost and the main thread never waits; each field stacks its owners, so an extension leaving (or a failed generation) puts back the next owner's value or the original, unless the field changed since.

- **`timeline.paint(painter)` (2026-11-08, additive):** a v4 extension paints on the timeline's clips, as v1–3 drew on its canvas. The painter gets the visible clips and returns per-clip shapes (rects, text) in clip points; it runs when the visible clips change (scroll, zoom, edits), never per playback frame, off the main thread, with 100 ms to answer. The timeline turns the shapes into its own ops clipped to each clip (TL-PAINT-1: p95 0.11 ms for 1,000 clips, optimized build); the JS side for 1,000 clips took ~12 ms in a debug realm, off the main thread. Shapes, not textures: v4 has no pixel canvas for painters yet.

## Edit lane decisions (2026-11-08)

- **On-canvas text** is an invisible P-TEXT canvas `NSTextView` (typing, IME, dictation, emoji, spelling) over engine-rendered text; the session is one kernel gesture ("Add Text" / "Edit Text"), the caret is the system `NSTextInsertionIndicator`, and frame, selection, misspellings and the marked-text underline are drawn from the engine's `textLayout`. Labels follow the kernel's title case.
- **Pen and Text overlays draw in GPUI over the paused viewer** (plan §3.3 static case); transform gizmos stay engine-drawn.
- **Viewer text questions** (caret geometry, picks, text boxes) go through one font-fed query mirror per project (`text_host::query`): missing fonts come from the registry, then the question runs again. Additive engine API: `Engine::add_font_bytes`, `font_outlines`, query `textFontOffset`; kernel `QueryContext::text_layout` / `text_font_offset`.
- **Canvas text input methods** place candidates at the engine's caret (`firstRectForCharacterRange:` answers the staged rect). Text at an angle gets its caret drawn along the glyphs; the system insertion point can only stand upright.
- **Shape variants** live in the toolbar Shape segment's menu; Q on the Shape tool cycles them (as on `main`); only Rectangle and Ellipse become masks.

## Look lane decisions (2026-11-08)

- **The owner's GPUI demo is the visual reference.** Translucent window (`translucency.ground` at `translucency.window` over a behind-window blur), one shared panel material (`colors.panel` at `translucency.panel`) with a `text` hairline edge, 8 pt panel radius, 38 pt headers with a divider, 24 pt status line. Reduce Transparency makes both solid; Increase Contrast makes edges solid `separator`.
- **Dark stays neutral grey** (`dark_is_neutral_grey`); the demo's warm stone hue is not adopted, only its materials and metrics.
- **Concentric radii apply only inside an outer corner's curve.** Panel content sits `inset` (12) in, past the 8 pt corner, so controls keep `control_radius` 6 (spec test states this rule).
- **Panel bodies never repeat `colors.panel`.** The v4 host drops a root `colors.panel` fill; panel windows (Export, Settings, Agent) paint it once.
- **v4 additive:** `metadata.placeholder` on `TextField` shows a muted hint in the empty field (no new prop).
- **The ground tint is darker than `colors.window` in dark (`#080808`, the demo's `#0C0A09` relationship)** because the blur lightens it; panels then composite a step above the ground, as tonal elevation requires.
- **The shell tab strip is the only Project / Effect Controls switch.** Both panels' own `SegmentedControl` (kept for the web host) is hidden in the native header.
- **Synchronous presents (live resize, key-window change) release the `CAMetalDisplayLink` first;** Core Animation forbids `nextDrawable` while a link owns the layer. While a link does own it, a draw with no link-supplied drawable waits for the link's next update and never falls back to `nextDrawable` (integration, wave 15).

## Compat lane decisions (2026-11-08)

- Extensions load from the migrated `extensions-shared` checkpoint plus the native `extensions` folder; the shipped app's live folder is never read.
- The trusted realm's first document is staged as JSON text outside JavaScript time; synchronous engine replies get their own size cap (`Limits::max_engine_response_bytes`, additive in runtime-v8).
- `PM.proj` is a write-tracking view; whole-tree diffs remain only for unflushed writes during native publications and for a replaced project root.
- Contributed panels build into a body inside a host-owned root, hidden until placed; a panel the shell restored opens when its definition registers.
- The page gets the compat theme edition matching the window's tokens (scheme, Increase Contrast, Reduce Transparency).
- Trusted-realm watchdog follows D11: Wait / Stop sheet, never an automatic stop; Stop disables only an extension caught activating.
- Each extension activates in its own realm task, so the watchdog times one extension.
- Additive runtime-v8 API: `Limits::queue_fetches` (fetches wait for room and leave room for one engine reply; Store helpers keep failing).
- Records an extension inserts read back as themselves; the realm's `structuredClone` unwraps project views.
- The realm's first document becomes its live tree (no second copy); the first publication is staged model-to-text.
- Shell `panel:park:<id>`: closes a tab but a later `panel:show` puts it back in place (used when an extension reloads).
- ae-workspace-theme turning itself off on a fresh profile is its own one-time recovery (storage key `restore-request-1.0.4`), as in Electron; a migrated profile that carries the key keeps it on.

## Extension authoring (moat lane, 2026-11-07)

- **v4 additive API:** an extension may export `mount(env)` (`BuiltinEnv`, the built-ins' own entry) as well as `view()`; it may import `powermove/{runtime,protocol,host-api,components,memo,renderer,builtin}`; `activate({hot})` and `env.hot` carry `api.hot.data` across reloads; a v4 manifest's `replaces` naming a built-in panel makes it a fork drawn in that panel's place.
- **v4 additive API:** `activate({commands})` and `env.commands.register({id,title,key}, run)` put an extension's commands in the window's palette and keymap; they run in the extension's realm and leave with it.
- Built-in sources ship embedded and are versioned `<package major.minor>.<content hash>`, so any change to a panel is a new version for `stage_fork_rebase`.
- The credential scan also reads `.tsx` and `.jsx` (main's list predates v4 sources).
- Agent change history lives in `<profile>/agent-change-history/<project>`; Undo is the palette's Undo Extension Change (newest change set only, as on main).
- The per-window isolate admission budget is 64 × 64 MiB (the plan's 64-isolate budget); 640 MiB admitted one extension beside the nine built-ins.
- **v4 additive API:** `SegmentOption.symbol` (optional SF Symbol name) draws a glyph segment with the label as its tooltip and accessible name.

## Open, recovery and quit (core lane, 2026-11-08)

- A file project's library entry is its row plus `projectMeta.file`; the `.pmv` is its only body, so open and save never copy it into the library. Entries written by older builds lose their copy on next open. A closed entry's Duplicate and rollback compaction read the file; when the file is gone, opening or duplicating the entry recovers its journal's work as a library copy.
- A journal session that starts from exactly what its file or slot holds writes checkpoint 0 at its first change, not at open; until then recovery replays from that base, including its undo history and workspace. A save deletes the sessions it made redundant at once.
- A journal whose document, layout and undo history equal the saved file retires without asking; any other difference asks.
- SIGTERM runs the normal Quit path; a clean quit releases `library.lock` (`ShellCall::ReleaseLibrary`, internal).
- Library projects read media the first launch moved out of IndexedDB from `<profile>/Media Store` (by `storageKey`, then `media:<fingerprint>`, then asset id) when their file lacks it; media that only Undo can bring back is read too. Library saves don't copy store-backed media into a backing file; Save As embeds it, as v1 did, except bytes the first launch classified proxy-only: those are marked proxy-only in the media table, export as proxies only, and never enter a file.
- Reopening a library project unchanged rewrites only its row and view state, never its slot.
- A record that a development-server IndexedDB origin also holds migrates from the shipped app's origin (`app_powermove_*`, captured first).
- The window shows a project as soon as the kernel opens it; its media catalog follows from the background refresh.
- MCP render_frames draws proxy-only media from its proxy, as the viewer does; export_video still refuses it (plan §6).
- A journal whose document and layout equal the saved file and whose undo steps are the saved steps merged into consecutive runs (an agent's finish_session squash after save_project, compared operation by operation), with nothing to redo on either side, also retires silently. The file's finer steps stay; nothing from the journal is adopted.
- While an open waits on a person's recovery choice, MCP tools that need the project say which project and that nothing changed; list_projects and the Store tools still run. The choice stays the person's.

## Playback and pro media (perf lane, 2026-11-08)

- Text coverage geometry is resolved in the outline's own space per quarter-octave scale bucket (flattened at the device tolerance over the bucket top, at most 2^(1/8) more lines); the path cache is 32 MiB, since a text-heavy 1080p frame needs about 10 MB at the coverage tolerance (wave 16 perf).
- Level 1 text measurements are memoized per font setup and text for a session (cleared with it), as Blink's per-font shape cache; `CanvasMeasurer::shape` returns `Arc<Measured>`.
- Pro media settings (Create Proxies, size, format, storage location) live in `<profile>/Pro Media/settings.json`, not the v1.2.0 profile keys; Always Play Original is per project and asset in the pro media index (machine-local intent).
- Background Tasks exposes priority through a right-click NSMenu (High/Normal/Low), as plan section 1 asks; rows keep queue order (the design review's "don't reorder" stands).
- Automatic plays optimized media in place of originals (Final Cut's optimized media); Originals and exports never read it; Export › Use Proxies (Draft) is the only way an export reads proxies.
- `pm_protocol::priority` gains `ProxySize::Full` and `ProxyFormat::ProResHq` (additive; bindings regenerated).
- Pro media renditions are keyed by the original's content (media-cache object name or a once-per-stamp BLAKE3), not its path and mtime; Clear… keeps anything another original, attached record, task or the open project references (wave 16 review).
- The direct glyph cache charges and evicts whole chunk textures, and still prefetch counts decoded bytes from the worker's decode and drops stills no frame asked for within 4 s (wave 16 review).
## Extension backends (compat lane, wave 16)

- Realm images decode natively on the realm worker: ImageIO for rasters (embedded profile to sRGB, EXIF orientation, as WebKit shows them), resvg for SVG, rasterized at the scale it is drawn (×2 for Retina) instead of stretched. An SVG's intrinsic size is usvg's (`viewBox` when width/height are missing), not Chromium's 300×150 fallback.
- The realm gains the web platform's `Blob`, `File`, `FileReader`, `URL.createObjectURL`, `atob`/`btoa`, `Image`, `ImageBitmap` and `createImageBitmap`; a realm `blob:`/`data:` image reaches the page as its bytes, never as a URL.
- While a project opens and until its panels have drawn it, the app holds a user-initiated activity (idle system sleep allowed), so App Nap never throttles an open in the background; measured on the T3 project with the screen locked: open finished in ~70 s instead of not finishing after 13 min at priority 4.
- Built-in panels lock only their own broker entry, and wait for host replies with a backoff (1 ms ×4, then doubling to 16 ms) instead of polling every 1 ms.
- `assets.import` from an extension imports into the Project panel without a layer, as v1; bytes the realm made are written to a private temporary file first, a picked file never is.
- Raster overrides answer per frame from the window's realm: the viewer never waits (a missing answer re-renders when it lands); exports wait per layer up to 60 s, then fail the frame rather than draw without the override.
- `render.raster`'s base result is lazy and writable, as main: only the untouched base of the asked layer, frame, scale and crop passes the layer through; nothing is painted unless an override reads it.
- An export's overrides answer from its frozen snapshot (section 17: edits never reach a running export); reloading or replacing the wrappers mid-export fails it rather than mix generations.
- A capture of the realm's own project answers the overrides it needs inline while the realm waits (the same-realm continuation), instead of deadlocking.

## Agents lane decisions (2026-11-08)

- **Panel and workspace tools run on the main thread** (`app/src/panel_bridge.rs`): the MCP worker sends the call and waits up to 30 s (main's timeout text); the main thread never waits for the worker. The headless `--powermove-mcp` host has no window and refuses them before parsing arguments.
- **Panel actions join the run transaction as main's `panelActions`**: interact/computer use refuse when the project moved since the run's last read, rollback refuses, and finish without commit after a change keeps the edits with main's message.
- **Bridge calls stop when the worker gives up** (30 s): a cancelled call stops at its next step, and an interrupted drag still sends its mouse-up.
- **Coordinate input checks what is under each point** in a probed frame (controls record bounds only while the bridge draws): protected, disabled and link targets refuse; type and press need an editable field; an attached sheet refuses.
- **Refs and input stamps are fenced by the panel's realm generation**, so a reload or fork swap that keeps the view invalidates old refs.
- **Captures say what they omit** (`omitted`): the viewer's and scopes' engine picture and WebKit extension bodies; cropping and PNG encoding run off the main thread.
- **Bridge calls draw a frame first** (not presented), so an occluded window or a locked screen still builds panel views, geometry and pointer listeners; input steps are paced one frame and 16 ms apart, as main paces drag points.
- **capture_panel draws one unpresented frame with the Reduce Transparency materials** and reads it back (vendored GPUI `frame-capture` feature ungates `Window::render_to_image`), so glass never shows what's behind the window. Images are PNG, like native `render_frames`, at most 1600 px wide.
- **interact_panel click activates the control the way a click does** (`SurfaceView::activate`); pop-ups and colour wells refuse click and point to `select` / `computer_use_panel`. Fill sets the field text, sends `input`, writes a bound field's model, then commits.
- **set_panel_layout saves a named shell layout off the main thread, shows it, and makes it the default for new projects** (`LayoutStore::default_layout`); a taken name gets a number; names allow 80 characters, as main. No Undo entry yet: kernel external history entries resolve through realm continuations and the shell has no realm (open item).
- **Thread tools run on the window's Agent panel, off the main thread** (`Builtins::agent_thread_control`): new threads and new turns take the caller's access (an unknown caller gets planning); a caller never steers a busy thread with broader access; a same-settings send steers the live run (or answers its blocking reply question); a changed provider, model or effort stops the run and starts a new turn; `clientRequestId` retries return the first answer and conflicting reuse refuses (in memory, per window, until quit). thread_watch/unwatch and task orchestration stay refused until runs can resume the calling agent.
- **Reasoning effort is per model** (`Catalog::model_efforts`): the effort picker lists the selected model's own efforts; switching to a model without the current effort falls back to Default, and asking a model for an effort it lacks is refused.
- **Agent attachments follow main's layout**: Add Attachments opens an Attach sheet; files (20 MB each, six per message) are read off the main thread into main's records (images as data URLs, text as content, else base64); a run writes images into its workspace for the provider (Codex `localImage`, Claude image paths) and passes other files as ATTACHED FILES in the prompt. Attachments can't join a running turn yet. The composer's selection is in text coordinates (attachments sit above the field).
- **v1–3 themes reach native chrome (plan 3.6.5):** the trusted compat realm runs the shipped theme translator (`js/themes`, bundled into the realm) on the active theme whenever contributions publish or `theme:changed` fires, and publishes `nativeTheme` (dark and light token patches, blockers). The app applies the patch for the window's appearance to every window's chrome and native panels; WebKit panels keep the theme's CSS. A v4 palette theme chosen later replaces it until the v1 theme changes. Surface rules beyond tokens are not applied natively yet.
- **A project without a stored workspace gives v1–3 extensions the shipped default ("design") workspace**, as `PM.WS.current` always held one in v1 (the compat adapter publishes it). Themes that edit `workspace.chrome` or `layout.docks` on activation failed `publish_extensions` validation before this.
- **User realms are third-party extensions and forks:** menus, clip painters and viewer-overlay frames reach both; painters are asked only in realms that registered one, under one 100 ms deadline per request, and their answers are validated and bounded on the worker before the main thread sees them.
- **A failed generation takes down what it put up** (overlay, chrome, painter, themes, commands) when its isolate fails (a quota, a crash); a closing window's realms just end, leaving the project's chrome as it is.
- **v1–3 themes keep a patch per appearance** (`pm_shell::theme::set_schemes`), so switching between light and dark needs no new publication, and a theme is re-applied only when its translation changes (timeline surface included), once for all windows.
- **Composer typing is fenced by the draft, not by every republish:** a whole-text draft from an older publication applies while the draft still reads what the person saw or what their typing last set; a draft the host cleared (a send) or replaced stays fenced. A late input event the renderer admitted for the same field lands on the current tree.
- **A panicking host operation ends its generation** (the V8 callback catches the panic and terminates execution) instead of aborting the process, which a panic can't unwind through V8's frames to avoid.
- **Agent flows are a repeatable script** (`apps/native/scripts/agent-flows.py`, stdlib Python against a running lane app): each publishes a synthetic extension through MCP, checks its effect in a panel capture, removes it and checks it's gone; a baseline that already shows the effect fails the flow. Flows: v4 theme, v4 viewer overlay, v1 and v4 timeline chrome, v4 clip painter, v1 theme.
- **The compat realm has the editor window's size** (`innerWidth`/`innerHeight`/`outerWidth`/`outerHeight`/`devicePixelRatio`, 1440×900 until the window reports; a change fires `resize`), replayed before every boot. v1–3 extensions size docks from it on activation; without it the co-founder's After Effects theme wrote NaN dock sizes on a first run, failed activation and never applied at launch.
- **The native timeline follows `workspace.chrome.timeline`** (v1–3 extensions and v4 `timeline.chrome`): row height → `track_row`, gutter width → `layer_column` (a column the person never dragged follows it), ruler height, keyframe size (fractions kept, as main) and clip radius (`clip_radius`, capped at 5 points as main's clip bars); surface order swaps track and gutter. Layer numbers, type badges and toolbar density are stored but not drawn natively yet (v1–3 writes them through the workspace).
- **`titlebar:right` contributions are accepted and not shown, as on main** (main's titlebar renders only its fixed buttons). v1–3 had no toolbar-item API; tool strips were panels, which v4 has.
- **Theme surface rules beyond tokens start with the timeline:** the translator's timeline inputs (track, gutter, accent, text, line, per scheme) are published with the v1–3 theme and drawn by the native timeline; a reversed `timelineSurfaceOrder` swaps track and gutter even without a theme (T1). Other surface rules (per-panel and per-control styles) are not applied natively yet.
- **Closed built-in panels park** (`Builtins::park` / `unpark`; Project, Effect Controls, Properties, Effects): closing one retires its isolate off the main thread and returns its reservation; opening it again starts a new generation that the kept view adopts; a fork that replaced the panel starts again as the fork. The viewer, timeline, agent, Settings and Export never park.
- **One cage account across windows is not adopted yet:** reserving every isolate at its heap ceiling in one process-wide 4 GiB account (built-ins 9 × 64 MiB per window, the compat realm 1 GiB per window, the extension compiler runner 1 GiB while it works) refuses a second window with extensions and fails 19 tests that run several hosts at once. It needs growth-based reservations (an initial slice per isolate, raised through V8's near-heap-limit callback only while the account has room), which is open.

## Design pass (design lane, 2026-11-08)

- Dev launch arguments `--appearance light|dark` and `--accessibility increase-contrast,reduce-transparency,reduce-motion` force the scheme or settings for that process only (the existing `PM_NATIVE_APPEARANCE` override plus `PM_NATIVE_ACCESSIBILITY`), so review captures never touch a person's system settings.
- `Integration::status_trailing` (default empty) gives the status line its format facts; the shell stays free of document state.
- New tokens: `metrics.titlebar` (28, plain panel windows' title band) and `metrics.popover_radius` (16).
- Clip selection is the accent ring and row selection fills the layer column only (owner's demo), replacing language §11's `text` ring.
- Review overrides are development-build only; a release build treats `--appearance`/`--accessibility` as unknown arguments and ignores `PM_NATIVE_ACCESSIBILITY`.
- A workspace `gutterWidth` of 224 (v1's written default) takes the native timeline column default; other widths are kept as authored.
