# Independent v4 API evaluation

Package 14 implements the 30 public briefs in `tasks.ts` as separate extension
modules in `../examples/tasks/`. Owner briefs remain `pending-owner`. These
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
