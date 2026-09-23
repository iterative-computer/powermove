# @powermove/registry

The contract between the desktop app and Powermove Cloud for the extension
Store. Runtime-neutral: every module under `src/` except `src/node/` runs in
Node, Bun, Electron main and Cloudflare Workers, using only Web Crypto,
`CompressionStream` and `TextEncoder`.

- `manifest` — the hand-written manifest parser (no imports; shipped to the
  agent as one file through the api-pack).
- `snapshot` — the one snapshotter: folder → git tree hash. Used for upload,
  modification detection and install verification.
- `git` — loose-object codec (blob, tree, commit), SHA-1 and SHA-256.
- `tar` — deterministic tar writer and validating reader.
- `limits` — registry caps and upload envelope.
- `scan` — credential scanner for publish and promotion.
- `diff` — tree comparison statuses and counts.
- `wire` — zod request/response schemas, DTOs and the `ApiError` union.
- `node` — `node:fs` walker producing snapshot inputs (desktop only).

Rules: `manifest.ts` imports nothing. Everything else may import zod and each
other. Nothing here imports from `apps/*`.
