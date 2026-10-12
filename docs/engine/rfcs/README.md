# Engine RFCs

One file per contract (plan section 27). Each contract is a set of Rust types
in `engine/crates/pm-protocol` with serde derives, a MessagePack encoding
(contract 9), and generated TypeScript in
`apps/desktop/src/shared/engine-protocol/`. The RFC states what the types
mean, the invariants that types alone cannot express (checked by
`pm_protocol::Validate`, which the decoder runs on every message), how the
contract is versioned, what each Phase 0 spike must confirm before it
freezes, and what is still open.

| # | Contract | Module | RFC | Gates | Freezes at |
|---|---|---|---|---|---|
| 1 | `EvaluatedScene` | `scene` | [01](01-evaluated-scene.md) | Phase 1 | Phase 1 start |
| 2 | `FrameRequest` and `FrameDescriptor` | `frame` | [02](02-frame-request-and-descriptor.md) | Phase 2 | Phase 2 start |
| 3 | The `pm-gpu` interop trait | `interop` | [03](03-gpu-interop.md) | Phase 2 | Phase 2 start |
| 4 | Color | `color` | [04](04-color.md) | Phase 2 | Phase 2 start |
| 5 | Patches (message envelope) | `patch` | [05](05-patches.md) | Phase 1 | Phase 1 start |
| 6 | The playback clock | `clock` | [06](06-playback-clock.md) | Phase 2 | Phase 2 start |
| 7 | Errors and isolation | `error` | [07](07-errors-and-isolation.md) | Phase 2 | Phase 2 start |
| 8 | Budget leases | `lease` | [08](08-budget-leases.md) | Phase 2 | Phase 2 start |
| 9 | Protocol codec and code generation | `envelope`, `codec`, `version`, `common` | [09](09-protocol-codec.md) | Phase 1 | Phase 1 start |
| 10 | Definition snapshots and lifecycle | `definitions` | [10](10-definition-snapshots.md) | Phase 2 | Phase 2 start |
| 11 | Blender jobs, snapshot, surface format | `blender` | [11](11-blender-jobs.md) | Phase 2 | Phase 3a start |
| 12 | Priority classes and agent frame requests | `priority` | [12](12-priority-and-agent-frames.md) | Phase 2 | Phase 2 start |
| 13 | GPU time slicing and deadline signals | `slicing` | [13](13-gpu-time-slicing.md) | Phase 2 | Phase 2 start |
| 14 | Host services | `host` | [14](14-host-services.md) | Phase 2 | Phase 2 start |
| 15 | The headless document host interface | `document_host` | [15](15-document-host.md) | Phase 2 | Phase 3c start |

"Gates" is from section 26 ("contracts 1, 5, 9 gate Phase 1"; the rest gate
Phase 2). A contract freezes when its first consumer phase starts.

## Process

- **Drafts are version 0.** Every contract reports version 0
  (`ContractId::current_version`). Peers negotiate drafts only when both sides
  were built from the same source (equal `build` strings in `Hello` and
  `Welcome`), so a draft can change without a bump while the engine and app
  ship together.
- **Freezing** sets the contract to version 1 in `version.rs` and in the
  table above. After the freeze, any change to a frozen contract's types or
  invariants needs an edit to its RFC, reviewed like code, and a version
  bump. Peers then negotiate the highest version both support
  (`version::negotiate`); a required contract with no common version fails
  the connection with `UnsupportedVersion`.
- **Wire changes are visible in review.** `pm-protocol-gen` regenerates the
  TypeScript types and one golden MessagePack fixture per message kind
  (`engine/crates/pm-protocol/tests/fixtures/wire/`). CI fails when either is
  stale, so every wire change shows up as a fixture diff.
- **Readers ignore unknown map keys** (serde's default). An added optional
  field is readable by older readers, but it is still a version bump after
  the freeze. New enum variants are never readable by older peers.

## Regenerating

```sh
source engine/env.sh && cd engine
cargo run -p pm-protocol --bin pm-protocol-gen          # write
cargo run -p pm-protocol --bin pm-protocol-gen -- --check   # verify only
cargo test -p pm-protocol                                # includes the no-op check
```
