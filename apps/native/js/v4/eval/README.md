# Independent v4 API evaluation

Package 14 implements the 30 public briefs in `tasks.ts` as separate extension
modules in `../examples/tasks/`. These
examples use package exports, hooks and UI components only; they do not access
`root.host`, import implementation modules, or copy `scenarios.tsx`.

The independent harness exercises behavior and replays emitted JSON commits into
the existing Rust wire example. It compares complete accepted snapshots, including
properties and listener flags, and tests rejection with the real validator.
The synthetic native input observation is modeled separately from wire commits;
the wire example cannot deliver AppKit input observations.
`runner.ts` retains its existing reconciler-comparison mode; independent evaluation
is the CLI default (`--extensions` is accepted for clarity). Use `--scaffold` or
`--candidate=...` to run the original fixture comparison.

All data is synthetic. All visual output is provisional. DESIGN-TODO(opus): the
three panel examples need design review; they specify behavior/data flow only.

The results distinguish brief conformance from missing production host services.
Headless passes do not certify macOS rendering, compatibility cutover or a freeze.

The ten owner-approved briefs now include behavior, inputs/interactions, required
host reads/writes, edge cases and checks in `tasks.ts`. Their synthetic fixtures
in `scenarios.tsx` run through public snapshots/events in every CLI mode. They
are authored models with explicit service seams, not independent extension ports.
`fixtureStatus` and `serviceStatus` describe executable headless checks. Every
owner run crosses HostClient -> Rust HostRouter/HostDataService -> synthetic
KernelData; project/selection/time/evaluated queries, initial and later
subscriptions, acknowledged atomic edits and settings are exercised. The
annotation fixture also drives these services and inspects the current native
text through the public snapshot. Domain/media logic in other fixtures still
uses explicit synthetic seams.

The CLI reports `status: pass` for this headless contract scope and retains
`productionReady: false`, `fullAcceptance: blocked-on-production-integration`,
`integrationGates` and `ownerProductionBlocked`. **40/40 headless passes do not
mean ten production owner panels are accepted.** The older static API-gap failure
classification is preserved in the historical evaluation report, not used as an
unconditional failure after executable data-service checks have been added.

Run `bun run eval --include-held-out` for all 40 records, and add `--wire` to build
and replay all commits through the standalone Rust surface validator. Native
host-service checks and negative protocol tests use real Rust validators in
both modes. `--wire=PATH` still selects an existing surface wire binary. Plain
`bun test` builds/configures the examples automatically; failed native builds
fail the suite. See `docs/native/v4-eval.md` for counts and the freeze hold.
