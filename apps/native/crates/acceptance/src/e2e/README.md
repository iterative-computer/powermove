# Frozen Playwright → native headless acceptance

Wave 5 e2e lane, frozen reference `v1.2.0` /
`07ed5fa17ee640659cb075f9cad1af2095011d60`. No old app or test was deleted.

The requested `git grep -l 'test(' v1.2.0 -- apps/desktop/e2e` returns **290
files**: **289 spec files / 614 test declarations** and one non-test fixture,
`fixtures/typewriter-effect/cursor.ts` (a regular-expression `.test(` match).
**80 declarations are parameterized**; these are declaration counts, not
expanded Playwright test-instance counts. The ledger retains the enclosing
loops, named parameter-table source, conditional skips and file-level guards.
None are executed or silently dropped during extraction.

`frozen.json` records every file hash and declaration's file/line, title,
registration and declaration hash. `backings.json` names the missing native
controllers/services/oracles for the existing W1 workflow mappings.
`traceability.json` links executable native checks to exact declarations and
states the assertion subset each check covers. Existing CSV rows link each
declaration to this ledger. The acceptance lane's 396-row file inventory,
acceptance IDs, supporting IDs, blocker IDs and file verdicts remain unchanged.
The standalone manifest follows `acceptance/runtime` and requires no changes
to the acceptance lane's `lib.rs`, manifest, workspace lock or runner.

## Results on Linux

**30 executable checks: 29 pass / 1 measured failure**, linked to **28 source
declarations in 26 spec files**. Of the **614 declarations**, **3 complete
headless equivalents pass / 1 fails / 610 remain blocked**. Passing domain
subsets and recording trait doubles never certify a UI/pixel declaration.
The three complete headless equivalents are the frozen revision recovery,
strip/library operations and selection-after-Undo/Redo scenarios. Their direct
PM/agent-tool assertions have native actor/MCP equivalents; this does not
certify their enclosing app workflow or macOS launch/input/transport.

`cargo test` has **36 tests**: the native checks and six ledger/report tests.
This harness count does not claim that 36 product contracts passed.

### Repaired Glide batch easing

`agent-keyframe-easing.spec.ts:5`, `set_property` variant: keys at 0/2/3 s with
values 10/110/−40 and `ease: 'glide'` must retain the complete curve. Before the
repair, native evaluation at 1 s gave **90.42094586353606**, expected
**104.94929544957891**. Both `set_property` and `replace_keyframes` now pass the
unchanged oracle. It independently inverts the frozen Glide handles
`[0.16, 0.84, 0.24, 1]` in f64 rather than reading native output.

The kernel finishes named presets against final neighbours at the end of each
uninterrupted property-command run. The former known-defect guard is an ordinary
passing test. The complete declaration retains its missing-controller blocker;
a passing supporting probe cannot certify complete application parity.

## Run

Every shell starts with the lane environment:

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
cd apps/native
cargo test --locked -p pm-acceptance
cd ../..
cargo test --locked --manifest-path apps/native/crates/acceptance/src/e2e/Cargo.toml
cargo fmt --manifest-path apps/native/crates/acceptance/src/e2e/Cargo.toml -- --check
cargo clippy --locked --manifest-path apps/native/crates/acceptance/src/e2e/Cargo.toml --all-targets --no-deps -- -D warnings
cargo run --locked --manifest-path apps/native/crates/acceptance/src/e2e/Cargo.toml --bin pm-e2e-triage > work/e2e/report.json
```

The JSON runner exits **1 for any measured failure**, **2 for unresolved
declarations**, **0 only for a passing requested scope**, and **64 for invalid
arguments/ledger**. The current full report exits **2** for unresolved declarations.
Checks run on a worker, including on macOS; no actor wait runs on main.
An individual native check is inspectable/drivable by its registered ID:

```sh
cargo run --locked --manifest-path apps/native/crates/acceptance/src/e2e/Cargo.toml --bin pm-e2e-triage -- --check easing_set_batch
```

Filtered results cannot certify unexecuted parameter variants. In particular,
running only `easing_replace_batch` passes that check but leaves its declaration
blocked because `easing_set_batch` was not executed.

Validate or regenerate provenance with development-only TypeScript 5.9.3 and
Python's standard library. Node/TypeScript is never a native runtime dependency:

```sh
npm install --prefix work/e2e --no-audit --no-fund --ignore-scripts typescript@5.9.3
NODE_PATH=work/e2e/node_modules node apps/native/crates/acceptance/src/e2e/extract.cjs
python3 apps/native/crates/acceptance/src/e2e/annotate.py
# Only after reviewing an intentional ledger change:
NODE_PATH=work/e2e/node_modules node apps/native/crates/acceptance/src/e2e/extract.cjs --write
python3 apps/native/crates/acceptance/src/e2e/annotate.py --write
```

## Decisions and selecting rules

| Decision | Rule |
| --- | --- |
| Use immutable tagged syntax, preserve every declaration/parameter/guard, and account for the regex-only fixture | N3 frozen intent; N4 no missing old tests; N6 retain v1–3 coverage |
| Execute native actor/MCP contracts by ID, with disposable synthetic profiles and real portable files | N7 headless/inspectable; N4 no user data touched |
| Treat three data/behavior-only scenarios as complete headless equivalents; keep UI/pixel subsets and trait doubles explicitly supporting | N3 correctness and evidence scope; N1 real professional behavior, not fabricated screenshots |
| Keep the failed full-curve oracle and publish its failing result; do not modify production paths owned by other lanes | N3 authored intent exact; ownership; quirk policy |
| Preserve fractional authored rates rather than old rounding | N1 professional rates; N3 exact timing; quirk policy |
| Use a standalone child manifest and annotate existing CSV rows only | Lane ownership; preserve parallel acceptance work and its existing consumers |
| Run checks on a worker; V8 logic/PM/sync services, native recorder pixels and remote-DOM WebKit panels remain the architecture | N8 no main-thread waits/Node/Chromium; N4/N6 unchanged extension contract |
| Retain removed standalone-web-export declarations as owner-scoped evidence, without reinstating the removed feature | Recorded owner decision; N4/prune: retain remaining player/portable-export consumers |

## Still to verify

The report's missing-capability entries are the backlog, tied to exact frozen
declarations. On macOS, connect actual GPUI/AppKit semantic surfaces and input,
V8 extension/service activation and WebKit remote DOM, original/proxy media
storage and decoders, Metal pixels/GPU recovery, audio devices, native export
children and installed MCP reconnection. Run frozen pixel/standards oracles and
paired physical timing/performance checks. Linux double results are not Metal,
WebKit, AppKit, audio, scanout, performance, or v1–3 corpus compatibility passes.

No visual/UX output or product copy was added. There are no new
`DESIGN-TODO(opus)` placeholders; all future visual work still requires the
existing Opus specifications and review.

## Parity follow-up — 2026-11-04

The merged shared triage lacks the historical per-declaration annotations this
runner expected. Exact links now live in owned `../../data/e2e-declarations.csv`;
`annotate.py` reads shared triage and writes only that owned map. Validation
compares all 614 IDs, paths, lines, hashes, titles, workflows and native checks.
All 396 specs and the frozen 370 roster remain unchanged. Run `annotate.py` to
check and `annotate.py --write` only after reviewing an intentional link change.

The strict Glide oracle is shared with `pm-acceptance::curve_oracle`; both reports
retain the same set_property failure and passing replace_keyframes variant.
The 36 Rust tests pass; 29/1 executable product checks and 3/1/610 complete
headless declaration results remain unchanged. See the dated section of
`docs/native/acceptance.md` for the 23 newly connected W1 cases and exact backing
groups. No source extraction or expected pixel/curve tolerance changed.

## Active main reference — 2026-11-04

The owner's current behavior reference is
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. The CLI and source extractor now default
to main; `--reference v1.2.0` on the CLI and `--reference=v1.2.0` on the extractor
retain the old evidence explicitly. The legacy inventory module/tests remain
compatibility evidence, not a current-main certificate.

Current main has 719 declarations in 314 specs; the exact source snapshot lives
in `../../data/main-e2e-frozen.json`. Both CLI entry points use the shared active
main acceptance report and `MAIN.*` IDs; the configured parent harness additionally
runs native V8 backing for the separately labeled compatibility census. The
unchanged main Glide declaration retains its measured failure; 718 main
complete declarations remain blocked. The union map retains 396 legacy rows and
the frozen 370 roster, adding one current spec. No original ledger was removed.

Run `pm-e2e-triage --reference v1.2.0 --check selection_preserves_redo` for the
old complete headless check, or `pm-e2e-triage --check MAIN.MCP.list_projects.catalog`
for a current schema-only check. Neither filtered result establishes app parity.

Existing non-MAIN check IDs still select their original compatibility source when no explicit reference is given; result metadata identifies that scope. Full/default reports use main.

## Easing batch repair — 2026-11-05

Both unchanged Glide operation oracles now pass. Native finishes named presets
against each property run's final neighbours, before a following operation can
read or override animation. The former known-failure guard is an ordinary test.
The complete main declaration retains its existing controller blocker; a
passing curve probe does not certify the full editor workflow. See
[the integration repairs](../../../../../../docs/native/int2-fixes.md) for the
remaining current-main failures and complete verification counts.
