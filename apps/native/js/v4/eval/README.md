# Independent v4 API evaluation

Package 14 implements the 30 public briefs in `tasks.ts` as separate extension
modules in `../examples/tasks/`. Owner briefs remain `pending-owner`. These
examples use package exports, hooks and UI components only; they do not access
`root.host`, import implementation modules, or copy `scenarios.tsx`.

The independent harness exercises behavior and replays emitted JSON commits into
the existing Rust wire example. It compares complete accepted snapshots, including
properties and listener flags, and tests rejection with the real validator.
`runner.ts` retains its existing reconciler-comparison mode; independent evaluation
is selected explicitly with `--extensions`.

All data is synthetic. All visual output is provisional. DESIGN-TODO(opus): the
three panel examples need design review; they specify behavior/data flow only.

The results distinguish brief conformance from missing production host services.
Headless passes do not certify macOS rendering, compatibility cutover or a freeze.
