# Extension comparisons — cloud handoff, 2026-11-04

Branch: `feat/na-cloud-extcheck3`, from `feat/na-extcheck2` at `9253bc087`.
Merged integration base: `711f44ba0667f81ce8388c4ec57565d1d0378630`.
**BLOCKED / P2 OPEN: 0 accepted matches, 0 measured mismatches, 58 can't run.**
The work is a tested continuation, not completion of handoff item 5.

The final integration refresh adds the 2026-11-04 owner decision moving overall
parity to `main` at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`.
This lane's frozen `v1.2.0` tool remains a **required legacy-input check** under
that decision and N4; it is not the latest-main oracle. The newly required
main-delta parity is also unverified. Do not silently relabel these recordings
or switch compiler/reference revisions without a separate audited pairing.

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
reference at `/tmp/pm-extcheck/reference`; if absent, create it with
`git worktree add --detach /tmp/pm-extcheck/reference v1.2.0`, then install its
frozen dependencies. Do not use an installed app/profile or another lane's
worktree. Provide the private reviewed script directory as
`PM_EXTCHECK_SCRIPTS`; neither names nor those scripts belong in Git.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export PM_S1_ARTIFACT=/private/tmp/pm-s1-v8-artifact-final
export CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4 TZ=UTC
: "${PM_EXTCHECK_SCRIPTS:?set the private reviewed scenario directory}"
node tools/extcheck/prepare-reference.mjs /tmp/pm-extcheck/reference
(cd /tmp/pm-extcheck/reference && POWERMOVE_TRACE_RECORDER=1 bun run --cwd apps/desktop build)
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
node --test tools/extcheck/test_clock_preparation.mjs tools/extcheck/test_startup_clock.mjs
PM_EXTCHECK_RUNNER="$CARGO_TARGET_DIR/debug/pm-compat-runner" node --test tools/extcheck/test_clock_v8.mjs
nice -n 20 python3 tools/extcheck/repeat.py \
  --reference-root /tmp/pm-extcheck/reference --store --repeats 3 \
  --scripts "$PM_EXTCHECK_SCRIPTS" \
  --output /tmp/pm-extcheck/cloud-continuation-reference-stability.json
nice -n 20 bash tools/extcheck/run.sh \
  --reference-root /tmp/pm-extcheck/reference \
  --native-runner "$CARGO_TARGET_DIR/debug/pm-compat-runner" --store --repeat 3 \
  --scripts "$PM_EXTCHECK_SCRIPTS" \
  --summary /tmp/pm-extcheck/cloud-continuation-summary.json
```

The wrappers retain the build/GUI leases, disk floor and owner pause/active
flags. Never remove another lane's lock or flag. Configure the existing
`ELECTRON_OVERRIDE_DIST_PATH` if the disposable reference uses an externally
prepared matching Electron binary. Until the endpoint/Store work above is
implemented, the final command is expected to exit **2 / P2 OPEN**.

## Final post-merge verification

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
