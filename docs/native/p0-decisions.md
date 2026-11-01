# P0: decisions and open items

Status 2026-11-01. Plan v6 is the overall implementation plan; not every
decision is Jude's. Technical calls within the plan's rules are made by the
orchestrator and recorded here; product calls go to the owners (Chike, Jude).
Each item names its evidence. Nothing here authorizes dropping a v1–3 member
(N4) or changing the frozen reference.

## Gates

| Item | State | Evidence |
|---|---|---|
| S1 V8 | Not started. The sandbox build needs far more disk than this Mac has (~10 GB free vs §9's 80 GB). Owner: a machine with space or CI. | — |
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
5. **Fallback** (trusted realm in V8 + remote DOM) is unimplemented and unmeasured; it depends on S1.

Decision needed (now firmer after the unlocked rerun): choose the scheme handler for page→app sync calls, and either accept S5 as "channels viable, pixel/continuation/background budgets need the engine-id design and an unlocked rerun", or trigger the V8 + remote-DOM fallback evaluation (blocked on S1).

## Orchestrator decisions (2026-11-01)

1. **Reference build:** tag `v1.2.0` (the plan's definition of "today"). The async render siblings added after the tag are supported too (a superset); traces come from v1.2.0.
2. **Authored vs fixed:** decided case by case under §4's quirk policy; each decision is recorded beside its fixture/trace.
3. **S5 direction:** page→app sync uses the scheme handler (0.10 ms p95); pixels stay in the engine and are referenced by id; a focused spike on continuation latency and background activity follows; the V8 + remote-DOM fallback is evaluated once S1 can run.
4. **Pro features missing at v1.2.0** (J/K/L, ripple, linked A/V, slide/roll, three-point editing) are cutover work, as §1 already states.

## Owner decisions (Chike, 2026-11-01)

- **Eval specs:** anonymised behaviour specs of Chike's panels may be used in the v4 eval (no names, ids or code in tracked files).
- **Store-wide counts:** pull published Store extension sources read-only through the Store tools; record usage counts only.
- **S1:** runs on a cloud/CI macOS runner (this Mac lacks the disk).

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
