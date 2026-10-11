# Powermove native engine

Rust workspace for the native engine. The design and plan are in
[`docs/rust-engine.md`](../docs/rust-engine.md).

```sh
source engine/env.sh        # pinned toolchain (rustup, see rust-toolchain.toml)
cd engine && cargo build
```

Crates map to the plan's crate table (section 4). Contracts live in `pm-protocol`
with RFCs in `docs/engine/rfcs/`.
