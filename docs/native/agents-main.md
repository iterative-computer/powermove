# Main-reference agent services

Reference: main at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. Merged from
`feat/na-cloud-agents3` (`595434b97`) on 2026-11-05. Code is the truth; see
[agents.md](agents.md) for the process and panel path.

None of that branch's modules remain. `compatible`, `completion`,
`markdown`, `media`, `visual`, `session` and `review` were merged, then removed
on 2026-11-06 because no production code called them. They stay in history at
`595434b97` for the Compatible provider (A01), reply markdown and local media
(A04), `publish_visual_reply` (A05), the session gate (A09) and review (A10).

## Not merged from that branch

- `memory`: superseded by `pm_model::memory`, `pm_mcp::memory_admission` and the
  kernel `Call::Memory` path.
- `registration`: `pm_mcp::registration` is the host for all nine clients. The
  branch's S070 launch-consent policy stays in its history and in
  `design/surfaces/agent-auth.md`, for whoever builds that alert.
- `jobs`: superseded by `pm_mcp::export_jobs`.
- `presentation`: superseded by `pm_panel_agentpanel::work_groups`.
- `fixtures/main/replay` (368 KB): its provider files were byte-identical to
  `fixtures/replay/providers` apart from the `baseline` label, and its catalog
  is already pinned by `mcp/fixtures/catalog-main-c7f46a056.json`.

## Fixtures

`fixtures/main/contracts.json` comes from executing main's source. Regenerate it
with `capture-main-contracts.cjs --check`, which needs Node and TypeScript 5.9.3
in `work/oracle/node_modules`. Only its Claude permission cases are still read.
