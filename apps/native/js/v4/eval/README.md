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
`fixtureStatus: pass` means those checks passed; `status: fail` with
`failureKind: api-gap` means full acceptance still needs the listed `blockedBy`
capabilities. Missing host services never become a fabricated task pass.

Run `bun run eval --include-held-out` for all 40 records or `--group=owner` for
the ten owner fixtures. The documented JS-only follow-up is 27 pass / 13 fail /
0 pending, including ten passing owner fixtures with full acceptance blocked.
See `docs/native/v4-eval.md` for per-task limits and wire-dependent failures.
