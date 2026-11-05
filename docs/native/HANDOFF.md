# Native rewrite — handoff to Jude

Snapshot: 2026-11-02, 03:27 UTC. This covers **all unfinished work**, including engine,
projects, panels, compatibility, design, performance, migration and release.
No builds, tests, captures or app launches were run to prepare this document.

The rewrite has a running Rust kernel, engine service, native shell and substantial
compatibility/v4 infrastructure, but **P1–P5 are not signed off**. The integration
source of truth is `feat/native-engine`, head
`3c4fbc4e543d64b87e337ac0965986bd9a76abfd`. Read
[p0-decisions.md](p0-decisions.md) for all owner decisions: plan adjustments,
the two latency endpoints, the frozen v4 API shape, design calls and panel calls.
[design/](design/) holds Opus's design language, specs and reviews;
[surfaces.md](surfaces.md) is the inventory of required surfaces and missing
backing. Passing models or headless tests do not establish a working, reviewed
Mac UI. The API shape is frozen; production integration and cutover remain open.

Frozen Electron behavior reference: `v1.2.0`,
`07ed5fa17ee640659cb075f9cad1af2095011d60` (see P0 decisions).

## Area handoffs — read these before continuing a lane

These links select the **working branch versions**, because these changes are
not yet on the integration head. Their newer findings supersede older diagnoses
in the integration documents. Four area notes arrived during the requested
wait. After the additional
60-minute wait, the dedicated extension note was still absent at the final
03:27:41 UTC fetch; its existing round-three report is used below.

| Area / branch | Handoff and concrete checkpoint |
| --- | --- |
| Engine/macOS — `feat/na-etrack-integrate` | [engine-macos.md](https://github.com/iterative-computer/powermove/blob/7ca9048a586c824fcbfa138f4ae93285b07a7087/docs/native/handoff/engine-macos.md): original engine branches, pro-media and export are merged here. Latest library replay: **908 pass / 0 fail / 2 ignored**; integration sweep incomplete; **AUDIO-2 still fails**; original frozen-corpus qualification remains. |
| Whole projects — `feat/na-corpus` | [corpus-p1.md](https://github.com/iterative-computer/powermove/blob/4aa346755f7dd6b127eec7521e7c1ac8044978b5/docs/native/handoff/corpus-p1.md): **26/58** passed all sampled stages historically. WIP now freezes the compiler alongside the worker; seven historical GLSL failures need re-diagnosis. Rotation and five media cases remain open. |
| Nine built-ins — `feat/na-builtins-live` | [builtins-live.md](https://github.com/iterative-computer/powermove/blob/e835967d12f944b4a6b2fba011783c0cdc903d1b/docs/native/handoff/builtins-live.md): all nine production v4 entries mount against the real kernel. Native presentation/actions are incomplete; final screen verification is pending. **140 native tests pass / 1 ignored**, but **21 Inspector snapshots fail** after symbol metadata changes. |
| Extensions — `feat/na-extcheck2` | [Current round-three report](https://github.com/iterative-computer/powermove/blob/9253bc087b6ce8066a3426ff8693d4970a4365cf/docs/native/extcheck-p2.md): deterministic clock tooling is isolated-test only; post-fix native transport build/tests and complete capture endpoints remain pending. **0/58 accepted matches**. Dedicated `docs/native/handoff/extcheck-p2.md` was not present at the cutoff. |
| 120 Hz — `feat/na-p120b` | [p120.md](https://github.com/iterative-computer/powermove/blob/55d396b62082671079f6006f43f782466c9037d6/docs/native/handoff/p120.md): `CAMetalDisplayLink` ownership explains the `nextDrawable` exception. Candidate patch is WIP; full rebuild, five-minute crash test and quiet pacing census remain. |

## Branch table

All fetched `origin/feat/na-*`, `origin/feat/ne-*` and `origin/feat/tr-prep`
branches with commits outside `origin/feat/native-engine` are listed below.
Ahead counts are **ancestry counts**, not independent changes; do not add them.
Last commit dates are UTC. “Ready” means ready to consume through its stated
integration route, not that a phase gate passed.

| Branch | Commits ahead | Last commit (UTC) | Purpose | Status | What to do |
| --- | ---: | --- | --- | --- | --- |
| `feat/na-builtins-live` | 1 | 2026-11-02 | Nine real v4 panel hosts and native presenter | WIP | Finish host actions/rendering, review snapshot changes, verify all nine on screen, then obtain Opus screenshot review. |
| `feat/na-cloud-etrack` | 219 | 2026-11-01 | Cloud integration of Jude's engine finish | Ready — carried by etrack | Already contained in `na-etrack-integrate`; use that lane and its Mac fixes. |
| `feat/na-cloud-export` | 5 | 2026-11-01 | Software delivery, HDR tags, archive-safe export | Ready — carried by etrack | Already contained in `na-etrack-integrate`; complete its remaining export/project checks there. |
| `feat/na-cloud-promedia2` | 5 | 2026-11-01 | Exact source-frame decoding, LUT/scopes jobs | Ready — carried by etrack | Already contained in `na-etrack-integrate`; finish native resource/UI and project verification there. |
| `feat/na-cloud-surf-follow` | 7 | 2026-11-02 | Approved panel/surface design follow-ups | WIP | Reconcile with built-ins, run native checks, connect application hosts, finish remaining review items and capture for Opus. |
| `feat/na-cloud-tl-render` | 4 | 2026-11-02 | Timeline ruler/track/keyframe/graph renderer | Needs design review | Reconcile overlapping built-ins timeline work; obtain Opus light/dark/Increase Contrast screenshot review, then merge. |
| `feat/na-corpus` | 5 | 2026-11-02 | Whole-project runner and blocker investigation | WIP — Jude | Prove frozen worker/compiler pairing, reduce shader/media/rotation cases, then rerun every local project. |
| `feat/na-etrack-integrate` | 237 | 2026-11-02 | Engine finish plus Mac/pro-media/export repairs | WIP — Jude | Complete unfiltered replay, AUDIO-2 and frozen parity gates; review changes and integrate the verified result. |
| `feat/na-extcheck2` | 1 | 2026-11-02 | P2 comparisons, round 3 deterministic references | WIP | Install the clock fixture in both hosts, prove repeatability, finish native capture/Store admission, compare all 58. |
| `feat/na-p120b` | 1 | 2026-11-02 | Metal drawable crash and 120 Hz pacing | WIP | Rebuild corrected size guard, validate lifecycle, then run quiet actual-present/CPU gates. |
| `feat/ne-bigrender` | 85 | 2026-10-23 | Jude's large-render/export work | WIP — carried by etrack | Already contained in `na-etrack-integrate`; verify there, no separate merge. |
| `feat/ne-l4comp` | 61 | 2026-10-25 | Jude's L4 compositing and reference gates | WIP — carried by etrack | Already contained in `na-etrack-integrate`; use original witnesses and pending ledger. |
| `feat/ne-l5stills` | 1 | 2026-10-25 | Jude's native-depth still-image path | WIP — carried by etrack | Already contained in `na-etrack-integrate`; finish source-depth/colour/palette parity there. |
| `feat/ne-phase4a` | 28 | 2026-10-22 | Jude's Phase 4a engine work | WIP — carried by etrack | Already contained in `na-etrack-integrate`; finish its gate evidence there. |
| `feat/ne-svg` | 34 | 2026-10-18 | Jude's SVG renderer and witnesses | WIP — carried by etrack | Already contained in `na-etrack-integrate`; qualify against original SVG witnesses. |
| `feat/tr-prep` | 148 | 2026-11-01 | Electron transition release, based on `main` | Needs owner OK | Separate Electron/main release path. **Never merge without explicit owner OK**; do not merge it wholesale into native-engine. |

Containment above was checked with `git merge-base --is-ancestor`. The engine
integration now includes all five original `ne-*` branches and all three named
cloud engine/pro-media/export branches. Branches with zero ahead commits are
already integrated and omitted. This documentation branch is based directly on
the integration head; it introduces no implementation merge.

To refresh the inventory after fetching:

```sh
for r in $(git for-each-ref --format='%(refname:short)' \
  'refs/remotes/origin/feat/na-*' 'refs/remotes/origin/feat/ne-*' \
  refs/remotes/origin/feat/tr-prep); do
  n=$(git rev-list --count "origin/feat/native-engine..$r")
  if [ "$n" -gt 0 ]; then
    TZ=UTC git log -1 --date=format-local:%F --format="$r | $n | %cd | %h | %s" "$r"
  fi
done
```

## Gate status: P1–P5

| Phase | What passes | What is blocked / next proof | Evidence |
| --- | --- | --- | --- |
| **P1 — native foundation, projects, migration and speed** | Kernel-open/edit/save-reopen passed for **39** admitted candidates; **26/58** passed every sampled stage. Engine library replay is 908/0/2; native app and pro-media targeted replays pass. Historical paired speed run completed only **5 pairs**, not the required 30. | No full project/parity/speed certification. **18 dataless inputs + 1 unavailable source**; seven GLSL cases need helper-bundle replay; five unresolved-media cases; one rotation guard. Engine integration replay, AUDIO-2, P-120, physical output, migration/restore and formal paired timings remain. | [perf-gate-p1.md](perf-gate-p1.md), [corpus report on its branch](https://github.com/iterative-computer/powermove/blob/4aa346755f7dd6b127eec7521e7c1ac8044978b5/docs/native/corpus-p1.md), engine/corpus/P-120 area notes above, [first-launch.md](first-launch.md), [s6-migration.md](s6-migration.md) |
| **P2 — legacy extensions, V8/remote DOM/Store** | **53/53 executable owner/local inputs activate** in real V8; component checks pass. Population remains **44 owner + 10 local + 4 Store = 58**. | **0 match / 0 measured mismatch / 58 can't run** as complete comparisons. Deterministic references, native script/trace/PNG endpoint, complete interaction scenarios, recorder/DOM/resource bindings and Store admission/dispatch remain. | [extcheck-p2.md](extcheck-p2.md), [round-three branch report](https://github.com/iterative-computer/powermove/blob/9253bc087b6ce8066a3426ff8693d4970a4365cf/docs/native/extcheck-p2.md), [compat-realm.md](compat-realm.md), [recorder-raster.md](recorder-raster.md), [store-host.md](store-host.md) |
| **P3 — v4 surfaces, themes and built-ins** | **40/40 headless eval tasks**, **172 Bun tests**, **95 distinct surface tests** on the recorded integration evaluation. All nine live entries mount on the built-ins branch. | **10 production owner-panel acceptances remain blocked**. Nine app panels still need complete native presentation/actions, snapshot repair and Mac/Opus acceptance. Remaining surface specs/builds, IME/AX/input/resource lifecycle and physical performance remain. API-shape freeze stands. | [v4-eval.md](v4-eval.md), [v4-api.md](v4-api.md), [surfaces.md](surfaces.md), built-ins area note |
| **P4 — agents and automation conformance** | Agent replay harness: **95 tests**, covering **23** recorded cases; reference regeneration is identical. Configured cross-phase acceptance: **1,439 pass / 0 fail / 3,159 blocked**. | Strict agent identity still fails on recorded observable differences. Real providers, cancellation/lifecycle/MCP error semantics, application queue ordering, orchestration and tool bodies remain. All **83 full workflow contracts** are blocked; **396 legacy specs**, including the frozen 370-spec roster, are retained. | [agents-replay.md](agents-replay.md), [agents.md](agents.md), [mcp.md](mcp.md), [acceptance.md](acceptance.md), [W1](w1-workflows.md#native-execution-status) |
| **P5 — pro editing/media and completion** | Ripple/roll/slide/source/link/shuttle domain operations and supporting acceptance probes exist. Native pro-media and export targeted checks pass on the engine lane. Packaging/handoff reports **128 component tests**. | Real editing gestures/source-monitor flows, media readiness/proxies/cache/tasks/LUT/scopes and complete codec/colour/audio/project output need application/physical proof. Signed updater/Keychain/migration/restore, transition owner approval and beta qualification remain. | [pro-editing.md](pro-editing.md), [pro-media.md](pro-media.md), [distribution.md](distribution.md), [first-launch.md](first-launch.md), engine area note |

The project runner's historical stage matrix is useful for triage, not a full
resolution or full-timeline visual oracle. One of the 26 all-stage passes retains
a capture warning, leaving **25 clean captures**. Counts are entries, not
deduplicated authored projects; **34 extension-dependent entries** were deferred
to P2 out of **92 discovered entries**.

| Sampled stage | Pass | Fail | Blocked |
| --- | ---: | ---: | ---: |
| Capture/open | 39 | 19 | 0 |
| Render | 28 | 7 | 23 |
| Play | 28 | 0 | 30 |
| Scrub | 28 | 0 | 30 |
| Audio mix | 34 | 1 | 23 |
| MCP edit | 39 | 0 | 19 |
| Save/reopen | 39 | 0 | 19 |
| MP4 + decode | 26 | 9 | 23 |
| ProRes + decode | 26 | 9 | 23 |

## Remaining work — ordered by phase

These are dependencies for finishing the rewrite, not claims of phase approval.
Bring verified lane results back to `feat/native-engine`; keep each blocker
visible until its actual assertion has been demonstrated.

### P1: engine, whole projects, presentation and speed

1. **Finish the engine integration.** Branch: `feat/na-etrack-integrate` (carries
   `na-cloud-etrack`, `na-cloud-promedia2`, `na-cloud-export` and all five `ne-*`).
   State: Mac repairs and targeted checks pass, but the 140-target integration
   sweep was interrupted; AUDIO-2 cold first buffer measured **11.053 ms** against
   **10.666667 ms**. Next: complete the unfiltered replay with Jude's frozen
   corpus, rerun `additional_integer_decoder_classes_are_drawn` after the TIFF
   repair, rerun all-target Clippy after the final fixture change, and fix the
   cold-audio deadline without relaxing it. Use the engine area note and
   [gate-policy.md](../engine/gate-policy.md); maintain the
   [Gate E ledger on this lane](https://github.com/iterative-computer/powermove/blob/7ca9048a586c824fcbfa138f4ae93285b07a7087/docs/engine/gate-e.json).
   Gate E still has **15 unqualified categories / 9 open work items**.
2. **Verify every project.** Branch: `feat/na-corpus`, then the verified engine
   integration. State: historical 26/58 all-stage passes, no new real-project
   result after the WIP diagnostics. Next: prove relocated worker + compiler can
   render/export generated glow/shadow; replay the seven shader cases, prove
   default-path rotated-video pixels before lifting the guard, reduce the five
   media-selection/binding failures, then run Jude's complete local project set
   with one frozen tool pair. Do not hydrate dataless files automatically.
   Follow the corpus area note and commands below.
3. **Fix the 120 Hz present crash, then pacing.** Branch: `feat/na-p120b`.
   State: standalone acquisition regression passes, full corrected patch is
   unbuilt; no new timing census. Next: rebuild the public `Pixels` conversion
   fix, run `--verify-drawable-lifecycle` and `--verify`, then
   `check-drawable-lifecycle.py --seconds 300` against the real release app.
   Verify actual Space/visibility transitions and responsiveness. Only after
   crash acceptance, run `check-ptext-120.py --display --no-build --runs 1`
   on a quiet Mac and investigate zero timestamps/missed intervals. Preserve
   the original workloads and CPU wall-time gate. See the P-120 area note,
   [s2-gpui-ce.md](s2-gpui-ce.md) and [s3-viewer.md](s3-viewer.md).
4. **Run the formal P1 speed gate.** Branch: tooling is on `feat/native-engine`;
   measure the integrated, verified candidate after the fixes above. State:
   diagnostic improvements exist, **no accepted speed gate**. Next: supply all
   missing native and Electron endpoints in
   [tools/perf-gate/README.md](../../tools/perf-gate/README.md), prepare the public
   workload/config, stop builds for the quiet window and run
   `PM_PERF_GATE_CONFIG=/private/tmp/pm-p1-config.json tools/perf-gate/run-gate.sh p1`.
   Require **30 accepted independent pairs per row**, matching frozen Electron
   on the same Mac, fixed display/quality, nominal thermal/AC/unlocked state and
   all ten rows. Use a 120 Hz XDR Apple-silicon MacBook Pro and the required
   common base-M-series 60 Hz rig; a build-Mac 60 Hz run is labelled a stand-in.
   Edit/scrub must report both **input→GPU completion p95 ≤9.333 ms** and
   **input→actual on-screen presentation beating paired Electron**; subtract no
   estimated compositor floor. Record the report in [perf-gate-p1.md](perf-gate-p1.md).

### P2: compatibility and the rest of the surface inventory

5. **Finish deterministic extension comparisons, then all 58.** Branch:
   `feat/na-extcheck2`. State: round-three clock fixture is not integrated into
   either driver; WIP `PM.setTime`/transport changes need build/tests/Clippy;
   previous all-source attempts had 56 unstable references and two reference
   activation failures. Next: validate the worker change, install the same
   deterministic clock/entropy fixture in both hosts, inspect **≥3 recordings
   per case**, and justify any volatility policy without hiding authored
   differences. Complete native script/trace/PNG and recorder→DOM presentation,
   Store release admission/resource/view/callback dispatch and reviewed full
   per-extension scenarios using the supplied projects. Then rerun all 58 and
   rank measured mismatch families. Keep each can't-run case in the denominator.
   Use [extcheck-p2.md](extcheck-p2.md), the newer branch report above and
   [tools/extcheck on that branch](https://github.com/iterative-computer/powermove/blob/9253bc087b6ce8066a3426ff8693d4970a4365cf/tools/extcheck/README.md).
   Also close the underlying runtime gates: macOS denial probes and final signed
   helper/JIT confinement, hard external-memory accounting/attribution,
   reconciler/cluster budgets, complete synchronous canvas/text/paint/encoding
   semantics and production media/resource adapters. Passing activation does
   not prove these. Use [runtime-v8.md](runtime-v8.md),
   [s1-v8.md](s1-v8.md), [compat-realm.md](compat-realm.md),
   [remote-dom.md](remote-dom.md) and [recorder-raster.md](recorder-raster.md)
   for their exact unclosed assertions.
6. **Spec and build the remaining surfaces.** Base: `feat/native-engine`, with
   `feat/na-cloud-surf-follow` and built-ins supplying existing foundations;
   no dedicated branches exist yet for the unstarted surface families. State:
   **54 P2/P3 surfaces need complete Opus specs**, plus the gaps across **7
   native-only surfaces**. Next: triage by the stable S/N IDs in
   [surfaces.md](surfaces.md), get Opus specs for missing states/interactions,
   implement real models/adapters/views, then review actual Mac output. Cover
   agent/auth/Store/publishing, captions/transcription, Easing, Shader Lab,
   Mixer/3D, workspaces/generated panels, Takes/Notes/Mods, onboarding, updates
   and diagnostics. Native-only work includes migration/consent/outcomes,
   colour scopes/LUT, tasks/storage, pro-edit tools and OS authorization outcomes.
   Existing “spec+model” rows still need their stated application bindings;
   a placeholder or a neighbouring spec does not close a surface.

### P3: real built-ins and design acceptance

7. **Finish loading and operating all nine built-ins in the real app.** Branch:
   `feat/na-builtins-live`. State: all nine entries mount; the accepted trees are
   only partly rendered and many platform effects remain unavailable. Next:
   re-open the latest binary and verify the compiled fixes for overlapping
   numeric wells, duplicate headers/timecode, opaque Viewer and missing symbols;
   review/regenerate the 21 Inspector snapshots and rerun affected suites.
   Finish Properties rows/services, real media readiness, Agent providers and
   anchored NSPopover, Settings services and Export planner/capability reporting,
   PMV/original closure, presets/tasks/cancellation/failure states. Also support
   app-level Agent/Settings before a project opens. Test floating-window reopen,
   switching, keyboard/AX/IME and large-list focus/virtualization. **Then obtain
   Opus screenshot review**: light, dark and Increase Contrast with selected,
   empty, loading/error and focused states. Read the built-ins area note and
   [panel specs](design/panels/README.md).
8. **Merge timeline rendering after review and finish design follow-ups.**
   Branches: `feat/na-cloud-tl-render`, `feat/na-cloud-surf-follow`,
   `feat/na-builtins-live`. State: timeline render code awaits the Opus screenshot
   review; surface follow-ups implement several approved answers but still need
   native checks/application attachment. Next: reconcile their overlapping
   renderer work, validate/capture the actual timeline and merge after approval;
   integrate surface changes, prepare its `followups_check.rs` driver and run
   `work/followups-check macos` for the native main-thread probe, then attach
   the real window/session/worker hosts. Reconcile every open
   **“Sol follow-ups”** item against current code rather than assuming old
   completed fixes are missing. Track each remaining item in its review:

   | Review group | Concrete outstanding checks / changes |
   | --- | --- |
   | [Kit](design/reviews/na-cloud-kit2.md), [Settings](design/reviews/na-cloud-p-settings.md) | Native secure field/Keychain, colour picker/sampler, menu measurement, wheel/momentum and fade adapters; replace supported placeholders, retain unsupported-setting policy. |
   | [Timeline](design/reviews/na-cloud-p-timeline.md), [Properties](design/reviews/na-cloud-p-inspector.md), [Effect Controls](design/reviews/na-cloud-p-effectcontrols.md) | Finish symbol/heading/text-area/drag and mixed-value AX recipes; timeline track/graph interaction and row navigation; error copy and native host actions. |
   | [Project](design/reviews/na-cloud-p-project.md), [Effects](design/reviews/na-cloud-p-effects.md), [Export](design/reviews/na-cloud-p-export.md), [Agent](design/reviews/na-cloud-p-agent.md), [Viewer](design/reviews/na-cloud-p-viewer.md) | Media grid/search/readiness/drag; real empty/error states; export retry/planning/preset failures; agent grouped steps/result kind/thread title and native composer/pickers; viewer controls/gizmos. |
   | [Authority](design/reviews/na-cloud-s-authority.md), [Dialogs](design/reviews/na-cloud-s-dialogs.md), [Documents](design/reviews/na-cloud-s-documents.md), [Editing](design/reviews/na-cloud-s-editing.md), [Library](design/reviews/na-cloud-s-library.md), [Media](design/reviews/na-cloud-s-media.md), [Recovery](design/reviews/na-cloud-s-recovery.md) | Check which approved answers already landed in surf-follow; finish native geometry/routing, kit Banner/EmptyState/ViewerGizmo adoption, real services and main-thread AppKit checks, including Esc/default focus and stale-target handling. |

   [design/review-checklist.md](design/review-checklist.md) and the whole
   [reviews directory](design/reviews/) remain the acceptance checklist,
   including earlier shell/theme/service/pro-media follow-ups.

### P4: agents, parity and e2e

9. **Complete agent/application conformance.** Base: `feat/native-engine`, then
   the built-ins branch for app/provider actions; no new repair branch is named
   yet. State: regression ledger matches known differences, but strict identity
   fails. Next: resolve cancellation completion, additive lifecycle events and
   MCP error content/validation precedence; run real pinned providers and native
   queues through question/approval/stop/reconnect, concurrent runs, durable
   history and orchestration/watch flows. Run the strict replay described in
   [agents-replay.md](agents-replay.md); use [agents.md](agents.md) and
   [mcp.md](mcp.md) for unimplemented bodies. Catalog/schema identity alone is
   insufficient.
10. **Close parity and e2e after the panels are live.** Base:
    `feat/native-engine` plus the verified lanes above. State: acceptance is
    **1,439 / 0 / 3,159** and all 83 full workflows remain blocked. Next: execute
    the configured [acceptance harness](acceptance.md), using real engine,
    app/kernel, native input/AX/IME, media/export and compat hosts; replace each
    blocked assertion with actual backing. Run the retained native replacements;
    use `bun run --cwd apps/desktop test:e2e:ledger` for the Electron reference
    ledger and targeted `test:e2e` cases as documented in
    [the engine e2e ledger](../engine/e2e-ledger.md). Those Electron runs do not
    certify native UI. Preserve all 396 triaged specs and
    the frozen 370 roster. Refresh the exact
    [remaining-blockers.csv](../../apps/native/crates/acceptance/data/remaining-blockers.csv),
    [e2e map](../../apps/native/crates/acceptance/data/e2e-map.csv) and
    [W1 status](w1-workflows.md#native-execution-status). An exit 0 with blockers
    is not a full pass; strict mode must expose them. Also resolve the recorded
    project-memory inventory/tag contract discrepancy rather than dropping it.

### P5: project speed/export, transition and beta

11. **Speed and export on every project, plus pro integration.** Branches:
    `feat/na-etrack-integrate`, `feat/na-corpus`, `feat/na-builtins-live`, then
    the integration branch. State: short 320×180 sampled exports and component
    tests exist; complete timelines and physical output are unverified. Next:
    take each locally available project through open/edit/play/scrub/audio,
    undo/save/reopen, full-duration target-resolution MP4/ProRes and supported
    alpha/HDR/depth variants; decode every output and compare timing, frames,
    colour, text, shaders and audio against frozen references. Measure startup,
    seek/edit/playback/export cost separately from the quiet formal P1 gate.
    Exercise source-monitor marks, linked A/V, ripple/roll/slide/shuttle,
    proxies/optimized media, render cache/tasks, LUT/scopes and cancellation/
    crash/low-disk cleanup. Finish the real media/resource/codec/host bindings;
    never report proxy-only bytes as originals. Report in corpus/engine ledgers,
    [pro-editing.md](pro-editing.md), [pro-media.md](pro-media.md) and
    [perf-gate-p1.md](perf-gate-p1.md).
12. **Transition release — owner approval required.** Branch: `feat/tr-prep`,
    built against `main`, not the native integration branch. State: preparation
    and retention/interrupt tests exist; actual switch/update gates remain
    disabled. Next: review
    [tr.md on its branch](https://github.com/iterative-computer/powermove/blob/3473905563a4091201db68ebe6131ee2b55d381f/docs/native/tr.md),
    verify signed two-DR Keychain ACL/identity, source/media closure and final
    delta, interrupted Squirrel/ShipIt handling and rollback, then present the
    concrete release candidate to the owner. **Never merge this branch without
    explicit owner OK.** A merge approval does not authorize tagging, publishing
    an update or dispatching a release workflow.
13. **Beta.** Base: verified `feat/native-engine`. Beta3 preparation supplies the
    service flat-file/source-link adapter, kernel import/folder admission,
    optional public-API Sparkle binding, isolated checkpoint recovery and
    headless signing proof tooling. See [current implementation and owner gates](beta2-readiness.md#beta3-implementation-preparation),
    [distribution.md](distribution.md), [first-launch.md](first-launch.md),
    [s6-migration.md](s6-migration.md) and [os-base.md](os-base.md).
    Application termination/import leases and actual signed installer/Keychain/
    notarization/physical proof remain open. Finish the application integration,
    native resource/rpath inventory, direct/skipped-TR and final-delta proof,
    consent/denial, interrupted saves/replacement, both architectures/macOS 14+,
    sandbox/JIT, TCC, Gatekeeper and isolated beta identity. Checkpoint recovery
    copies into a new private directory; installer recovery retains the old app.
    The owner requires **no reverse migration and no archive Restore UI**. The
    original app must refuse a native file it cannot fully read without changing
    it. Preserve original Electron data, unknown fields, history and archived
    copies. Complete prior gates and owner review before any authorized beta
    publication; this preparation authorizes no tag, upload or release.

## Mac setup, build, test and run

Use the branch for the task above. These commands are for Jude's continuation;
they were **not executed for this documentation change**. Run from the repository
root unless a command explicitly changes directory.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_BUILD_JOBS=4
# pm-media native build needs Python >=3.10; this is the build Mac's interpreter.
export PM_MEDIA_NATIVE_PYTHON=/opt/homebrew/bin/python3.14
"$PM_MEDIA_NATIVE_PYTHON" --version
mkdir -p /private/tmp/pm-native-tests
export TMPDIR=/private/tmp/pm-native-tests/
export DYLD_LIBRARY_PATH="$PM_FFMPEG_DIR/lib"
diskutil info /
```

Use an installed Python ≥3.10 if that exact Homebrew path differs. `env.sh`
selects the compiler/cache and FFmpeg environment; invoke the pinned compiler
explicitly (`cargo +1.96.1`) for native app work. The V8-enabled built-ins and
extension runner also require the verified S1 archive **and matching generated
bindings** from [runtime-v8.md](runtime-v8.md), not a random V8 download:

```sh
export RUSTY_V8_ARCHIVE=/private/tmp/pm-s1-v8-artifact-final/librusty_v8.a
export RUSTY_V8_SRC_BINDING_PATH=/private/tmp/pm-s1-v8-artifact-final/src_binding.rs
unset V8_FROM_SOURCE
```

Those temporary paths describe prepared build-Mac inputs, not durable installed
assets; recover/verify the pair if absent. Keep ≥8 GiB available before shared
builds; obey any stricter runner requirement throughout. Check APFS **Container
Free Space**, not `df` alone. Hold `/tmp/pm-build.lock` for every build/test;
one compile at a time, at most two local lanes. Do not clean another lane's
artifacts. Honour `PAUSE_LOCAL`; screen work also needs an unlocked Mac, absent
`USER_ACTIVE` and an exclusively owned `/tmp/pm-shots/gui.lock`.

Build/test the native app (on the built-ins branch, include `pm-builtins`):

```sh
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 build --locked \
  --manifest-path apps/native/Cargo.toml -p pm-native-app
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 test --locked \
  --manifest-path apps/native/Cargo.toml -p pm-native-app -p pm-surface
# feat/na-builtins-live only:
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 test --locked \
  --manifest-path apps/native/Cargo.toml -p pm-builtins
```

For panel Bun tests, first build the current wire examples under the build lock
and set `PM_SURFACE_WIRE` / `PM_UI_KIT_WIRE`; otherwise the test preload may run
Cargo outside the lock. The built-ins note supplies the exact commands and
debug-info settings. For acceptance, install the remote-DOM Bun dependencies,
then run `apps/native/crates/acceptance/scripts/verify.sh` under the build lock;
follow its macOS sandbox-worker configuration in [acceptance.md](acceptance.md).
Keep raw reports local; publish only reviewed public/synthetic aggregates.

Wrap the binary as a development app so AppKit gets the bundle name/accent:

```sh
PM_HANDOFF_SCRATCH="$(mktemp -d /private/tmp/pm-native-dev.XXXXXX)"
apps/native/scripts/bundle-dev.sh \
  "$CARGO_TARGET_DIR/debug/powermove-native" "$PM_HANDOFF_SCRATCH"
# After taking the GUI lease; select a disposable project COPY.
PM_NATIVE_PROFILE="$PM_HANDOFF_SCRATCH/profile" \
  "$PM_HANDOFF_SCRATCH/Powermove.app/Contents/MacOS/powermove-native" \
  "/ABSOLUTE/PROJECT-COPY.pmv"
```

The script recreates `Powermove.app` within its output directory, so use a fresh
scratch directory. `--profile PATH` is an alternative to `PM_NATIVE_PROFILE`.
Do not use the installed Electron app/profile as the native destination. The
bundle helper is development packaging, not a signed release or beta proof.

## Jude's first project-verification session

1. **Start on `feat/na-corpus` and prove the tool bundle.** Rebuild both worker
   and compiler with its area-note commands. Run the driver safety tests and
   generated glow/shadow checks; they must succeed with the relocated compiler.
   The WIP expectation changes are not proof of a fix. Keep the real failures
   and private source mapping outside Git.
2. **Run a folder of locally available test projects.** Choose the folder;
   embedded PMV media is supported, linked originals must already resolve.
   The empty profile below prevents accidental default-library discovery:

   ```sh
   source engine/env.sh
   export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
   export CARGO_BUILD_JOBS=4
   (cd apps/native && nice -n 20 flock /tmp/pm-build.lock \
     cargo +1.96.1 build --locked -p pm-native-app --example pm-corpus)
   (cd engine && nice -n 20 flock /tmp/pm-build.lock \
     cargo +1.96.1 build --locked -p pm-glsl --bin pm-glsl-compiler)
   PM_CORPUS_EMPTY_PROFILE="$(mktemp -d /private/tmp/pm-corpus-input.XXXXXX)"
   mkdir -p "$PM_CORPUS_EMPTY_PROFILE/store"
   nice -n 20 python3 tools/corpus/run.py \
     --binary "$CARGO_TARGET_DIR/debug/examples/pm-corpus" \
     --compiler "$CARGO_TARGET_DIR/debug/pm-glsl-compiler" \
     --profile "$PM_CORPUS_EMPTY_PROFILE" \
     --projects "/ABSOLUTE/PROJECT_FOLDER" \
     --batch-gib 1.5 --timeout 240 --retain-for-debug \
     --report /tmp/pm-corpus-folder-summary.json
   ```

   This samples open/render/play/scrub/audio/MCP edit/save-reopen and short
   MP4/ProRes decode, using private writable copies. It does not prove whole
   timelines, speaker sync, physical presentation or speed. Keep ≥6 GiB during
   batches. Resume only the same owned snapshot/profile/tool pair with
   `--resume-snapshot --resume-results`; a changed tool pair or incomplete
   discovery requires a fresh complete run for certification. Leave the 18
   dataless inputs unavailable unless the owner independently supplies local
   copies. Never promote proxies to originals.
3. **Then use `feat/na-etrack-integrate` with the original frozen witnesses.**
   Review the Mac repairs first. Set the full capture and component witness
   paths explicitly, build a same-checkout compiler, then run unfiltered gates:

   ```sh
   export PM_ENGINE_CORPUS="/ABSOLUTE/FROZEN_CAPTURE"
   export PM_ENGINE_CORPUS_ROOT="/ABSOLUTE/WITNESS_PARENT"
   # Set PM_MASK_REF / PM_SVG_FRAME_REF too if selecting explicit witnesses.
   (cd engine && nice -n 20 flock /tmp/pm-build.lock \
     cargo +1.96.1 build --locked -p pm-glsl --bin pm-glsl-compiler)
   export POWERMOVE_GLSL_COMPILER="$CARGO_TARGET_DIR/debug/pm-glsl-compiler"
   cd engine
   nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 test --locked --no-fail-fast \
     -p pm-audio -p pm-cache -p pm-color -p pm-eval -p pm-export -p pm-expr \
     -p pm-geom3d -p pm-glsl -p pm-gpu -p pm-media -p pm-protocol -p pm-proxy \
     -p pm-reference -p pm-render -p pm-wasm
   nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 test --locked --no-fail-fast \
     -p pm-node -p pm-vector
   ```

   Include evaluator, render parity, own layers, reused draws, text layout,
   masks/Canvas/SVG and preview/export identity. Use `--no-fail-fast` so a failing
   harnessless evaluator does not prevent later gates from running. Preserve
   exact evaluation/text/timing and pixel/geometry bounds from
   [gate-policy.md](../engine/gate-policy.md). Fresh M5 witnesses do not replace
   the frozen corpus or justify new pending fingerprints/exclusions.
4. **Capture only when a specific new witness is needed.** The ordinary engine
   corpus capture drives a built legacy app and uses a temporary profile. It
   records that build's hash; ensure it is the intended reference first. Example
   from the repository root, using synthetic input and no private default project:

   ```sh
   unset ELECTRON_RUN_AS_NODE
   nice -n 20 flock /tmp/pm-build.lock flock /tmp/pm-electron.lock \
     bun engine/corpus/src/capture.ts --label v1 --id v1-local-layers \
     --only layers/ --skip-real --skip-scenarios --root /private/tmp/pm-engine-corpus
   bun engine/corpus/src/verify.ts /private/tmp/pm-engine-corpus/v1-local-layers
   ```

   See [engine corpus instructions](../engine/corpus.md) for reference app
   preparation, Node/Electron bridge and explicit `--real` copies. Keep captures
   outside the repository. Label new captures honestly; retain original witnesses.
5. **Report and reduce.** Put only anonymized counts, fixed failure classes,
   exact code/tool SHAs, environment, gate verdict and reproduction commands in
   `docs/native/corpus-p1.md`, the relevant area handoff, engine Gate E/quality
   ledgers and the phase documents listed above. Keep every fail and blocker.
   Private mappings, project names/paths, media, traces, screenshots and logs stay
   local. Reduce confirmed defects to synthetic tests before committing them.
   Follow functional fixes with full-project speed/export and the quiet paired
   gate; update acceptance blockers after the real UI backing is connected.

## Known issues and gotchas

- **macOS `/var` and temp paths:** `/var` and `/tmp` are aliases for private
  paths. Safety checks can correctly refuse symlink spellings. Canonicalize test
  roots or use `/private/tmp/...`; keep production symlink protection intact.
  The corpus migration tests now canonicalize roots and report real ENOSPC.
  Their earlier interruption failure was low disk, not a reproduced migration
  regression. Re-run after the newest worker diagnostics.
- **Kernel calls off the Mac main thread:** blocking actor waits/drop and model
  wire queries must run on workers. Several old panel wire helpers panicked on
  macOS until moved off-main. UI callbacks enqueue work and consume published
  replies; never wait synchronously on JS or the kernel from AppKit/GPUI.
- **Bun version:** the kit review records **629/630**, with the portable-paths
  candidate-eval test hitting a Bun **1.3.14** internal error on `/private/tmp`;
  the v4 evaluation reports Bun **1.4.4**. Reproduce with the recorded version
  before changing assertions; keep the failing result visible until verified.
- **Shared target disk and freshness:** `$HOME/Developer/pm-target` can consume
  most available storage and reuse another checkout's named helper/binary.
  Build one lane at a time, verify the binary/helper identities and freeze a
  private tool pair per corpus run. Use consistent debug-info settings to avoid
  duplicate large artifacts; never delete another lane's target. The engine
  area note documents an identity path-remap salt for stale cross-checkout reuse.
- **P-120 crash is not yet closed:** a paused `CAMetalDisplayLink` still owns
  drawable acquisition. Missing callback drawable must not fall back to
  `nextDrawable`. The candidate patch needs full native lifecycle/resize/display
  checks. Zero `presentedTime` invalidates a timing census; do not substitute
  target times or silently omit zeros.
- **Stale MCP descriptor — fixed:** integration includes `c4a04c7a5`
  (`fix(native-mcp): archive stale host descriptors before relaunch`). Do not
  diagnose an old descriptor from a crashed host as a missing MCP implementation;
  preserve authenticated ownership and never remove a live host's descriptor.
- **GLSL helper packaging:** historical fallback reports may actually mean the
  frozen worker lost its adjacent `pm-glsl-compiler`. Use the new `--compiler`
  bundle and inspect real compiler diagnostics before changing guards.
- **Known real/open checks:** AUDIO-2 cold deadline; default rotated-video
  guard; media selection versus journal-applied assets; full frozen parity and
  pending fingerprints; incomplete native Export flags/PMV execution; live
  window/IME/AX/provider/Store bindings. Do not turn these into passes with
  broader tolerances, blank pixels, optimistic capability flags or skipped cases.
- **Existing harness issues:** the kernel selection-event test has a recorded
  setup/subscription race; a serial rerun passed. Whole-app formatting had an
  existing `app/src/smoke.rs` issue. AppKit probes that print SKIP off-main are
  not macOS UI proof. Preserve those distinctions in new evidence.

## Integrating and publishing evidence

Stage explicit paths, inspect the staged file list and diff, and run:

```sh
git config core.hooksPath .githooks
gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow
```

Keep hooks/scanning enabled. Public evidence must contain no credentials,
session descriptors/tokens, private projects, personal data or raw operational
artifacts. Record owner decisions in `p0-decisions.md`; Opus retains design
spec/review ownership. This handoff authorizes no release tag, published app
update or release-workflow dispatch. The transition branch separately requires
explicit owner OK before any merge.
