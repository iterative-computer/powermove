# Integration int1

## 2026-11-04 — ordered cloud-lane integration (in progress)

Base: `d7a0b01d430d7ec9977f52d784f047c027801b14` (`feat/native-engine`).
Behaviour reference: `c7f46a05641b23944d3692c9d80f9677c7e1ef1d` (`origin/main`).
Linux container, Rust 1.96.1. No macOS behaviour is certified here.
The missing activation script/toolchain was bootstrapped outside the repository.

Decisions: preserve both lanes' consumers and behaviour during conflict
resolution (N3/N4/N6; prune rule). Keep all failures and unavailable physical
checks explicit (N1/N2/N3). No new visual/interaction design is authorized;
existing Opus specifications remain authoritative.

| Merge | Incoming head | Conflicts / resolution | Affected locked test result |
| --- | --- | --- | --- |
| tokens6 | `e8a2f10f7282eb48208237f82d2789c9666cdc36` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-design`: 12 passed |
