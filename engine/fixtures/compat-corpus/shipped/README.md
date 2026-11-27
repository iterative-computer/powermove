# Today's shipped built-ins

The source of every built-in extension as the released app ships it
(`main` at 470eeaf3, before the native engine), without tests. The
compatibility corpus (`e2e/engine-customization-corpus.spec.ts`, owner
amendment 24) forks each of them the way `api.extensions.fork` does and
loads the forks unchanged with "Use New Engine" on. Never edit these files:
they stand for code users already have in their extension folders.

`timeline` is not included: its shipped `timeline.ts` trips gitleaks'
`generic-api-key` rule on an animation property assignment (the same
reviewed false positive `.gitleaksignore` lists for its earlier commits), and
a new file cannot carry a commit-bound ignore. Its fork fails to build on
today's app anyway (it imports `../../renderer/src/legacy/core/text-animation`,
which the extension compiler refuses outside the app).
