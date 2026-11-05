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
| surfa | `4f0743569b3e5690def2321f050f0f84503db5a7` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-services`: 195 passed, 0 failed, 0 ignored |
| surfb | `5576c1d9041da9fcdf6d316880725d48e6f7cf96` | Services exports/manifests and both locks: union dependencies; isolated lock updated only for pm-services. surfaces.md: retain complete dated A and B sections. | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-services -p pm-first-launch -p pm-promedia`: 408 passed, 0 failed, 0 ignored |
| fileguard | `64201add3726ec90c538b7a884ce8ae7b84a8b91` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-kernel`: 283 passed, 0 failed, 1 ignored |
| delta-data | `94bbf01911cc7524859f7c4edbfcef9af72ec7f6` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-kernel -p pm-native-migrate -p pm-first-launch --no-fail-fast`; `cargo test --locked --manifest-path engine/Cargo.toml -p pm-model`: 476 passed, 0 failed, 2 ignored |
| pro2 | `57d8ba570f02b8fdd4ae5c5f71a21a1898fb12d6` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-kernel`: 331 passed, 0 failed, 1 ignored |
| accept2 | `ce2b39a13b6a4c0ff3f8eb251fd69f3aa41cc9de` | No textual conflict. Corpus probe recognizes only the exact known fileguard stamp; all unknown fields stay exact (N3/N4). Updated nested locks for merged kernel/services dependencies. Initial stamp mismatch and missing remote-DOM dependency failed visibly; resolved and rerun with Bun 1.4.4/frozen install. | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance`; `cargo test --locked --manifest-path apps/native/crates/acceptance/runtime/Cargo.toml`; `cargo test --locked --manifest-path apps/native/crates/acceptance/src/e2e/Cargo.toml`: 139 passed, 0 failed, 0 ignored |
| agents3 | `1ee040ea738db41e8bbd3414d7d6bd200e5eb7db` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-agents`: 126 passed, 0 failed, 0 ignored |
| mcp3 | `f1b7c8b679deeaf80897c1b19f872d9d9a2d29af` | None | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-mcp`: 77 passed, 0 failed, 0 ignored |
