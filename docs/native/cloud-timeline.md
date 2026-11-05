# Cloud timeline integration

## 2026-11-05 — native action admission

Reference: `c7f46a05641b23944d3692c9d80f9677c7e1ef1d` (main).
Base: `5bcd425dd` (native-engine). The read-only `feat/na-cloud-tl-render`
head is already an ancestor of this base; its renderer is retained.

Decisions:

- N3/N4: join Position only when `separateDimensions` is explicitly false.
  Presentation groups channels without changing stored keys, values, expressions,
  unknown fields or exact legacy subframe times. Group selection expands only
  keys at exactly equal stored times. Axis diamonds still address one channel.
- N3/N7/N8: admit semantic gesture/action intents through the existing v4 worker
  dispatch and real kernel actor. Use replacement gesture steps and actor-owned
  commit/cancel/history. Native geometry supplies seconds/values and stable ids;
  JS never owns document state or converts a coordinate into an authored value.
- N3/N7: decimal document/revision stamps may fence native driver actions. Reject
  malformed ids, nonfinite numbers and missing required inputs before a lease.
- N1/N3: preserve errors and remaining platform boundaries. Headless actor tests
  do not qualify native hit testing, screen output, menu/field presentation,
  transport delivery or Opus review. No coverage row is fully closed by adding
  an action endpoint.

New native action types are declared beside the Timeline panel contract and
implemented in `pm-panel-timeline/src/v4_actions.rs`. They reuse the existing
commands and drag implementation. No extension contract or frozen MCP tool
catalog changes, and no consumer is pruned.

Verification and application-host boundaries are appended after integration.
