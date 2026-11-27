# Native acceptance harness

`apps/native/crates/acceptance` (`pm-acceptance`) checks the native app against
the workflows and keyboard map of the shipped Powermove 1 (`v1.2.0`). It runs
headlessly against the production kernel and MCP bodies, plus an optional native
worker (`pm-acceptance-runtime`) for cases that need V8, the compatibility realm
or engine-backed services.

## Inputs

- [w1-workflows.md](w1-workflows.md): the 83 workflows (W001–W083), their
  original cases and the native execution status table.
- [w1-keymap.md](w1-keymap.md): the shipped keyboard map, 195 bindings.
- [e2e-triage.csv](e2e-triage.csv): every v1 Playwright spec mapped to the
  workflows it covers.
- `apps/native/crates/acceptance/data/`: backings, catalogs and the e2e map the
  runner reads, plus synthetic fixtures. Fixtures are public, synthetic data
  only; the harness refuses user-supplied corpora in report refreshes.

The catalog embeds the first three files at build time
(`src/catalog.rs`), so a change to them is a change to the harness.

## Case results

Every case is `pass`, `fail` or `blocked`. A blocked case names the missing
backing (a service, host or platform surface not yet available headlessly) and
is never counted as a pass. Supporting probes and test doubles are reported
separately from the original workflow cases, and a filtered run says that it is
not a full-workflow verdict.

## Running

From the repository root:

```sh
apps/native/crates/acceptance/scripts/verify.sh
```

Install the remote-DOM build dependencies first (`bun install --frozen-lockfile`
in `apps/native/js/remote-dom`). The script runs both acceptance test suites,
formatting and Clippy, builds the compatibility bundle under
`work/native-acceptance`, and writes `report.json` and `remaining-blockers.csv`
there. It exits 0 when nothing failed (blockers allowed), or 2 with `--strict`
when blockers remain. It accepts filters such as `--case W037.compat-services`
or `--workflow W033 --strict`.

The pieces can also be run on their own:

```sh
cd apps/native
cargo test --locked -p pm-acceptance
cargo run --locked -p pm-acceptance -- --workflow W033 --format markdown
cargo run --locked -p pm-acceptance -- --format e2e-csv
cargo run --locked -p pm-acceptance -- --format catalog
cargo run --locked -p pm-acceptance -- --format blockers-csv
cargo test --locked -p pm-acceptance-runtime
```

The JSON report (`powermove.acceptance`, version 1) carries each case's kind,
status and reason, the blockers, macOS-only flags, workflow aggregates and the
legacy e2e mappings. Exit codes: 0 no failures, 1 an assertion failed, 2 blockers
under `--strict`, 3 invalid configuration.

## Refreshing status

After a full configured run, refresh the status columns from the report:

```sh
python3 apps/native/crates/acceptance/scripts/refresh-reports.py work/native-acceptance/report.json
cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance -- --format e2e-csv > apps/native/crates/acceptance/data/e2e-map.csv
```

The refresh script refuses filtered reports and edits only the e2e status
columns, the W1 status table and the blocker snapshot. Regression tests reject
drift between the report, the e2e map, the triage file and W1.

## macOS-only checks

Hit testing, focus, IME and accessibility, menus, clipboard and dialogs,
windowing, WebKit panels, physical display and audio timing, and the signed
sandbox can only be verified in the running app on a Mac. Cases that need them
are flagged as macOS-only in the report rather than passed headlessly.
