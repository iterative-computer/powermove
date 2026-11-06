# Main-reference agent services

Reference: main at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. Merged from
`feat/na-cloud-agents3` (`595434b97`) on 2026-11-05. Code is the truth; see
[agents.md](agents.md) for the process and panel path.

| Module | Row | State |
|---|---|---|
| `compatible`, `completion` | A01 | Address rules, presets, discovery and SSE parser. Real HTTP and Keychain hosts are open. |
| `markdown` | A04 | Inert block and inline trees for replies. |
| `media` | A04 | Rooted file grants, ranges and bound reads. |
| `visual` | A05 | `publish_visual_reply` input and limits; viewer isolation is open. |
| `session` | A09 | Owner/lease gate. `McpSession` retires replaced documents itself. |
| `review` | A10 | Frozen settings and one-use repairs. No review orchestration caller yet. |

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

`fixtures/main/contracts.json` and `provider.json` come from executing main's
source. Regenerate with `capture-main-contracts.cjs --check` and
`capture-main-provider.cjs --check`; they need Node and TypeScript 5.9.3 in
`work/oracle/node_modules`. The contract recorder was not re-run after its
sections for superseded modules were filtered out.
