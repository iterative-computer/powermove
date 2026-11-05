# Extension comparisons — cloud handoff, 2026-11-04

Branch: `feat/na-cloud-extcheck3`, from `feat/na-extcheck2` at `9253bc087`.
Merged integration base: `711f44ba0667f81ce8388c4ec57565d1d0378630`.
**BLOCKED / P2 OPEN: 0 accepted matches, 0 measured mismatches, 58 can't run.**
The work is a tested continuation, not completion of handoff item 5.

Active behavior/compiler reference: **`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`**
(current main, owner change applied). The tool now defaults to this pin, with
explicit `--reference-revision` selection for legacy
`07ed5fa17ee640659cb075f9cad1af2095011d60`. Current and legacy captures cannot
cross compiler/reference revisions; full legacy acceptance has the separate
`LEGACY_PASS` label. The public corpus produced 27 repeatable native component
recordings under each compiler pair (54 total), not full visual acceptances.

## What is implemented

- The worker PM.setTime/transport change builds and its regressions pass;
  rejected running-clock seeks now leave transport state unchanged (**N3**).
- A shared clock/entropy source is installed before trusted native code and
  prepared before frozen trusted/Store renderer modules. Identical raw source
  bytes, seed and SHA-256 avoid Node/Bun function-serialization differences.
  V8's frozen performance object, timer tasks and remote-DOM animation-frame
  ownership are covered. Native Store helper preludes execute while confined.
  The reference uses external local scripts under the unchanged shipping CSP;
  startup/lifecycle progress reaches all frames. No clock state/pixels/rows are
  rewritten to manufacture agreement (**N3/N6**).
- Full comparisons require three recordings for each role; an unstable second
  sample does not prevent collection of the third. Diagnostics include
  interactions and the existing exact visible-pixel rule. Population accounting
  retains missing inputs and source-read failures in 44 + 10 + 4, and reports
  drift/extra cases (**N4/N6**).
- Portable host tests and native probes run on Linux. AppKit/WebKit examples
  require `macos-probes`; `build.sh` sets it for the smoke example. Linux uses
  upstream V8, while macOS retains the verified sandbox archive/bindings path.
  Worker APIs remain separate from main-thread rendering (**N7/N8**).

No visual design or UX change was introduced; no new DESIGN-TODO(opus).
Cargo dependency versions did not change; both lockfiles remain unchanged and
all Cargo verification uses `--locked`.

## Evidence and limits

The public corpus produced **9 repeatable native component probes**, with
**three complete identical recordings each (27 total)**. The tenth public
case is rejected by the frozen manifest parser. Each probe observes activation,
clock/document/transport, runtime errors, queued DOM mutations, seek and
disposal. This is not a full behavioral trace or WebKit/PNG oracle. Raw copies
and recordings were removed after each case; only aggregate evidence belongs
in this repository. The synthetic real-V8 scheduler test separately inspected
three identical recordings. The frozen reference builds successfully on Linux;
the emitted trusted and Store documents load the identical fixture asset before
module code. No Electron window or WebKit page ran here.

The anonymous public Store catalog enumerated four releases. All four
archive/tree/file/hash inventories verified, and a second catalog read found
no drift. The owner input root and reviewed private projects/scenarios are
unavailable here. The full Linux comparison attempt preserves **58 can't run**:
44 missing owner inputs and 14 cases blocked for actual macOS capture, with
both roles attempted for each available case. Native probe success does not
reduce this denominator or turn into an accepted match.

Completed command counts are appended below after the final integration fetch.
One unresolved validation issue must stay visible: during concurrent headless
validation the durable Store-kernel profile-reopen regression intermittently
returned `Library(AsCopy(Locked))` (**23 pass / 1 fail**) three times, including
the `ce36cb6fc` post-merge sweep. The strict assertion was retained; its temporary profile is now canonicalized and the
failure includes the stage and synthetic lock record. Subsequent isolated and
full uninstrumented runs passed 24/24; eight strace runs also passed. The
`ce36cb6fc` sweep again failed at **reopen**, with a **PID 0** lock record.
The final `711f44ba0` post-merge sweep passed 24/24. The root cause is
**not established**. Do not interpret the later passing sweep as a
fix. Further investigation may require a change outside this lane in
`apps/native/crates/kernel/src/locks.rs` / `persistence.rs`; those files were
not edited. Do not add retries, serialize cases, widen the assertion or unlink
lock files to make this pass (**N3/N4**, admission/data correctness).

## Remaining implementation, before a P2 acceptance run

1. Complete the native script/behavior-trace/PNG endpoint. The bundled adapter
   still executes activation/disposal and reports
   `native-webkit-script-adapter-unavailable`; it is not an integrated full
   app capture. Integrating a full-app screenshot/script entry also needs shell
   wiring in `apps/native/crates/app`, outside this lane's owned paths. Connect
   real main-thread WebKit presentation through the
   existing nonblocking host/router boundary, actual input, trace instrumentation
   and snapshot completion. An admitted DOM batch is not a paint acknowledgement.
2. Complete recorder-to-DOM canvas presentation and the required unsupported
   recorder/text/paint/encoding/media resources; preserve explicit failures.
3. Connect Store release admission and resource/view/callback dispatch. Public
   hashes are not a native signed-release envelope. This container has no
   preserved installer provenance, pinned publisher envelope, or owner grants;
   none was fabricated. Public Store bytes never enter the trusted facade.
   Further implementation is needed, not just a Mac rerun (**N4/N6/N8**).
4. Supply the frozen input snapshot and authored projects, review complete
   extension-specific scripts, then inspect at least three **full** recordings
   per role/case. Default smoke scripts remain incomplete. Keep all 58 cases,
   including unavailable and invalid-manifest cases, and rank only actually
   measured mismatch families. Verify reference startup, CSP/Store iframe
   fixtures, fonts/scale/window geometry, native WebKit paint/input and native
   recorder pixels on macOS. No Linux pass establishes those behaviors.
5. Resolve the intermittent profile-reopen admission failure without relaxing
   the contract. The final branch signal is BLOCKED, not MERGE-READY.

## Exact Mac continuation command

Run in this branch from the checkout root. Reuse an **owned disposable** frozen
reference at `/tmp/pm-extcheck/reference-main`; if absent, create it with
`git worktree add --detach /tmp/pm-extcheck/reference-main c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, then install its
frozen dependencies. Do not use an installed app/profile or another lane's
worktree. Provide the private reviewed script directory as
`PM_EXTCHECK_SCRIPTS`; neither names nor those scripts belong in Git.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export PM_S1_ARTIFACT=/private/tmp/pm-s1-v8-artifact-final
export CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 TZ=UTC
: "${PM_EXTCHECK_SCRIPTS:?set the private reviewed scenario directory}"
node tools/extcheck/prepare-reference.mjs /tmp/pm-extcheck/reference-main
(cd /tmp/pm-extcheck/reference-main && POWERMOVE_TRACE_RECORDER=1 bun run --cwd apps/desktop build)
bash tools/extcheck/build.sh test
bash tools/extcheck/build.sh test pm-compat-host
bash tools/extcheck/build.sh test pm-store-host
bash tools/extcheck/build.sh clippy
bash tools/extcheck/build.sh clippy pm-compat-host
bash tools/extcheck/build.sh fmt
bash tools/extcheck/build.sh fmt pm-compat-host
bash tools/extcheck/build.sh build
python3 -m unittest discover -s tools/extcheck -p 'test_*.py'
node tools/extcheck/test_clock_fixture.mjs
node --test tools/extcheck/test_clock_preparation.mjs tools/extcheck/test_startup_clock.mjs tools/extcheck/test_references.mjs
PM_EXTCHECK_RUNNER="$CARGO_TARGET_DIR/debug/pm-compat-runner" node --test tools/extcheck/test_clock_v8.mjs
nice -n 20 python3 tools/extcheck/repeat.py \
  --reference-root /tmp/pm-extcheck/reference-main --store --repeats 3 \
  --scripts "$PM_EXTCHECK_SCRIPTS" \
  --output /tmp/pm-extcheck/cloud-continuation-reference-stability.json
nice -n 20 bash tools/extcheck/run.sh \
  --reference-root /tmp/pm-extcheck/reference-main \
  --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --store --repeat 3 \
  --scripts "$PM_EXTCHECK_SCRIPTS" \
  --summary /tmp/pm-extcheck/cloud-continuation-summary.json
```

The wrappers retain the build/GUI leases, disk floor and owner pause/active
flags. Never remove another lane's lock or flag. Configure the existing
`ELECTRON_OVERRIDE_DIST_PATH` if the disposable reference uses an externally
prepared matching Electron binary. Until the endpoint/Store work above is
implemented, the final command is expected to exit **2 / P2 OPEN**.

## Pre-reference-change post-merge verification

Base merged before this sweep: `711f44ba0667f81ce8388c4ec57565d1d0378630`.
Counts below are independent suites, not unique extension acceptances.

| Exact command (checkout root) | Result |
| --- | --- |
| `bash tools/extcheck/build.sh build` | Runner builds with `--locked` |
| `bash tools/extcheck/build.sh test` | **24 pass** on the final `711f44ba0` sweep; earlier **23 pass / 1 FAIL** profile-reopen runs remain unresolved |
| `bash tools/extcheck/build.sh test pm-compat-host` | **20 pass** |
| `bash tools/extcheck/build.sh test pm-store-host` | **90 pass** |
| `bash tools/extcheck/build.sh clippy` | Clean, `--locked --all-targets --no-deps -- -D warnings` |
| `bash tools/extcheck/build.sh clippy pm-compat-host` | Clean, same flags |
| `bash tools/extcheck/build.sh fmt` | Clean |
| `bash tools/extcheck/build.sh fmt pm-compat-host` | Clean |
| `python3 -m unittest discover -s tools/extcheck -p 'test_*.py'` | **38 pass** |
| `node tools/extcheck/test_clock_fixture.mjs` | **1 script pass**, including three entropy recordings |
| `node --test tools/extcheck/test_clock_preparation.mjs tools/extcheck/test_startup_clock.mjs` | **8 pass** |
| `PM_EXTCHECK_RUNNER="$CARGO_TARGET_DIR/debug/pm-compat-runner" node --test tools/extcheck/test_clock_v8.mjs` | **3 pass**, three actual V8 recordings |
| `node tools/extcheck/test_bundle.mjs /tmp/pm-extcheck/reference "$CARGO_TARGET_DIR/debug/pm-compat-runner"` | **1 script pass**, including exact Node/Bun fixture bytes and real shared controls |
| `node tools/extcheck/test_reference_profile.mjs` | **1 script pass** |
| `bun run --cwd apps/native/js/compat test` | **177 pass** |
| `bun run --cwd apps/native/js/compat typecheck` | Clean |
| `cargo build --locked --manifest-path apps/native/crates/remote-dom/standalone/Cargo.toml --example wire` | Wire oracle builds |
| `PM_REMOTE_DOM_WIRE_BIN="$CARGO_TARGET_DIR/debug/examples/wire" bun run --cwd apps/native/js/remote-dom test` | **39 pass** |
| `bun run --cwd apps/native/js/remote-dom typecheck` | Clean |
| `PM_TRACE_REF=/tmp/pm-extcheck/reference bun test tools/trace-recorder/tests` | **9 pass**, 1,518 assertions; no skip |
| `(cd /tmp/pm-extcheck/reference && POWERMOVE_TRACE_RECORDER=1 bun run --cwd apps/desktop build)` | Frozen reference builds; identical external fixture emitted for trusted/Store documents |
| `python3 tools/extcheck/probe.py --reference-root /tmp/pm-extcheck/reference --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --output /tmp/pm-extcheck/cloud-native-probes.json` | **9 repeatable / 1 invalid-manifest**, 27 native recordings; exit **2** |
| `python3 tools/extcheck/run.py --reference-root /tmp/pm-extcheck/reference --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --store --repeat 3 --summary /tmp/pm-extcheck/cloud-summary.json` | **0 match / 0 mismatch / 58 can't run**, exit **2** |

The first remote-DOM JS run contended with the Cargo build lock and exceeded
its unchanged five-second case deadline. Prebuilding the locked wire oracle
and running with `PM_REMOTE_DOM_WIRE_BIN` passed all 39 under the original
limit. The first trace-recorder run used its default missing reference path
and skipped the overlay case; setting `PM_TRACE_REF` to the prepared owned
reference passed all nine. No timeout, assertion, pixel tolerance or case
selection was relaxed. The profile-reopen failure remains unresolved and is
one reason, alongside unfinished capture/Store/scenario implementation, why
this branch must not receive a MERGE-READY finish signal.


## Owner reference change — current commands and delta scope

The active Mac command above now prepares the pinned-main checkout at
`/tmp/pm-extcheck/reference-main`. Its paired run is current-main, not a relabelled
legacy run. For an independent required legacy input check, reuse an owned
legacy checkout and pass its revision explicitly:

```sh
node tools/extcheck/prepare-reference.mjs /tmp/pm-extcheck/reference-legacy 07ed5fa17ee640659cb075f9cad1af2095011d60
# Build that checkout with POWERMOVE_TRACE_RECORDER=1 before full capture.
python3 tools/extcheck/probe.py --reference-root /tmp/pm-extcheck/reference-legacy \
  --reference-revision 07ed5fa17ee640659cb075f9cad1af2095011d60 \
  --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" \
  --output /tmp/pm-extcheck/legacy-native-probes.json
```

`repeat.py` and `run.py` take the same explicit legacy revision. Never point the
default current-main request at the old compiler. Source/pair fences and reports
are covered by `node --test tools/extcheck/test_references.mjs` and the expanded
**40-test** Python suite. The build-tool-to-V8 regression passes independently
against each checkout; each public corpus pairing has **9 repeatable cases ×
3 recordings**, with **1 invalid-manifest case retained**. The pinned-main
reference builds successfully on Linux. Current-main full capture remains
**0 / 0 / 58 can't run**, not a qualification.

The X-area delta audit is recorded in the new dated section of
`../extcheck-p2.md`. A08 theme/fork sources and A09 publication/panel-generation
sources are byte-identical across the pins; these remain required native carry
work. No native lifecycle acceptance is inferred from those facts. Retirement
and publication ledger carries remain under the open 58-case gate. E-row effect
helpers/metadata/capabilities go to delta-fx; U-row interaction/presentation goes
to the later UI lane; P rows go to delta-data. No optimistic effects capability
was enabled to activate a main-only consumer. The original endpoint, recorder,
Store, scenario and profile-lock blockers above remain open.

At the new pin, the worker transport also needs real presented-frame feedback
for the changed input-preview pause path (`legacy/core/engine.ts`): pause uses
the last displayed preview only while project/content generation still match.
The headless worker currently has no attached renderer providing that receipt.
Basic clock/seek/loop tests pass; presented-preview pause remains an explicit
E18/transport integration gap. Do not report the basic tests as that behavior.

Post-reference-change verification (same merged base, all Cargo commands locked):

| Exact command | Result |
| --- | --- |
| `bash tools/extcheck/build.sh test` | **24 pass**, earlier intermittent admission failures unresolved |
| `bash tools/extcheck/build.sh test pm-compat-host` | **20 pass** |
| `bash tools/extcheck/build.sh test pm-store-host` | **90 pass** |
| `python3 -m unittest discover -s tools/extcheck -p 'test_*.py'` | **40 pass** |
| `node --test tools/extcheck/test_clock_preparation.mjs tools/extcheck/test_startup_clock.mjs tools/extcheck/test_references.mjs` | **11 pass** |
| `PM_EXTCHECK_RUNNER="$CARGO_TARGET_DIR/debug/pm-compat-runner" node --test tools/extcheck/test_clock_v8.mjs` | **3 pass** |
| `node tools/extcheck/test_clock_fixture.mjs` | **1 script pass** |
| `node tools/extcheck/test_bundle.mjs /tmp/pm-extcheck/reference-main "$CARGO_TARGET_DIR/debug/pm-compat-runner"` | **1 script pass** |
| `node tools/extcheck/test_bundle.mjs /tmp/pm-extcheck/reference "$CARGO_TARGET_DIR/debug/pm-compat-runner"` | **1 legacy script pass**; this existing owned checkout is at the legacy pin |
| `bun run --cwd apps/native/js/compat test` | **177 pass** |
| `PM_REMOTE_DOM_WIRE_BIN="$CARGO_TARGET_DIR/debug/examples/wire" bun run --cwd apps/native/js/remote-dom test` | **39 pass**, original timeout |
| `PM_TRACE_REF=/tmp/pm-extcheck/reference-main bun test tools/trace-recorder/tests` | **9 pass**, 1,564 assertions, no skip |
| `python3 tools/extcheck/probe.py --reference-root /tmp/pm-extcheck/reference-main --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --output /tmp/pm-extcheck/main-native-probes.json` | **9 repeatable / 1 invalid**, **27 recordings**, exit **2** |
| `python3 tools/extcheck/probe.py --reference-root /tmp/pm-extcheck/reference --reference-revision 07ed5fa17ee640659cb075f9cad1af2095011d60 --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --output /tmp/pm-extcheck/legacy-native-probes.json` | **9 repeatable / 1 invalid**, **27 legacy recordings**, exit **2** |
| `python3 tools/extcheck/run.py --reference-root /tmp/pm-extcheck/reference-main --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --store --repeat 3 --summary /tmp/pm-extcheck/main-summary.json` | **current-main**, **0 match / 0 mismatch / 58 can't run**, exit **2** |

The same runner/host all-target Clippy commands (`--no-deps -- -D warnings`)
and fmt checks were rerun cleanly after the reference change. Both native
lockfiles remain unchanged. Main-reference preparation/build succeeds; actual
Electron and WebKit execution, complete authored scenarios, preview-frame
feedback and the production endpoint/Store work are still unverified/incomplete.
