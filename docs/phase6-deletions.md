# Phase 6 deletion manifest

Lane TESTS ports the surviving behavioral oracle coverage before any legacy
files are removed. This manifest is the deletion boundary for Lanes B and C.

## Lane status

- ENGINES: complete. The Svelte shell, layout, overlays, and panels are
  unconditional; the legacy chrome engines and their persisted/query escape
  hatch are retired. Media import now identifies mov/mp4 codec failures and
  gives ProRes `.mov` files an explicit transcoding warning.
- TESTS/TREE: the top-level deletions below are handled independently from the
  renderer-engine cleanup. The Vitest suite and its committed fixtures are the
  surviving behavioral oracle.

## May be deleted

After the replacement gates pass, Lanes B/C may delete exactly these paths:

- `tests/` — only after Lane C confirms the Vitest ports and removes the
  temporary `test:oracle` package script.
- `js/`
- `native/`
- `index.html` at the repository root
- `scripts/build-macos-app.sh`
- `spikes/probe/`

The Swift-extraction half of the former agent-prompt oracle dies with
`native/`; `src/main/codex/instructions.test.ts` now compares the shipped
TypeScript instruction and result-schema builders directly with their goldens.

## Must survive

- `spikes/fixtures/` — Playwright end-to-end coverage consumes these fixtures.
- `tests-vitest/fixtures/` — committed Vitest goldens, DOM contract data, and
  the TypeScript-engine performance baseline live here.
- `src/renderer/src/legacy/__tests__/` — behavior-level oracle transplants and
  their real-installer `makePM` harness.

Do not delete the whole `spikes/` tree: only `spikes/probe/` is approved.
