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
| beta2 | `f3b2186c905ca09b34d1df0eaa654b3921a982a3` | snapshot.rs: retain capacity and retention modules. Resolved services/first-launch/distribution cycle with pm-update-policy and API reexports (N3/N4/N8). Connected real verified disk-media adapter; keep malformed/unindexed inputs refused with unchanged archives (N3/N4). New negative regression passes; 15 bundle-audit Python tests pass. | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-first-launch -p pm-native-migrate -p pm-services -p pm-distribution -p pm-update-policy --no-fail-fast`: 425 passed, 0 failed, 0 ignored |
| extcheck3 | `5c7b8cbf6e63592bcda9425ef44d83ab67305bb5` | None. Required frozen JS dependencies and real pm-store-helper/pm-compat-runner binaries built before final replay. Additional checks: 40 Python, 11 Node reference/startup, 3 real-V8 clock, 177 compat JS tests pass. | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-compat-runner -p pm-compat-host -p pm-store-host`: 134 passed, 0 failed, 0 ignored |
| delta-fx | `1079e76940432bc7e45b647c94e1c1ed2e75dd79` | No textual conflicts. Preserve all etrack2/engine work and data/effect rows. Clean incoming rustfmt/EOF differences. 5 Python inventory tests and pinned main regeneration pass. The unfiltered evaluator corpus explicitly remains blocked on CoreText/frozen witness. | `cargo test --locked --manifest-path engine/Cargo.toml --no-fail-fast -p pm-fx-defs -p pm-color -p pm-eval -p pm-expr -p pm-geom3d -p pm-glsl -p pm-cache -p pm-protocol -p pm-reference -p pm-wasm -p pm-stretch`; `cargo test --locked --manifest-path engine/Cargo.toml -p pm-export`; `cargo test --locked --manifest-path engine/Cargo.toml -p pm-proxy --no-default-features`; `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-promedia`: 787 passed, 0 failed, 3 ignored; 1 macOS corpus target BLOCKED (exit 1), not a pass |
| 3d-core | `1ffa79bda4137ef7872d6688f681b70a592e65c4` | No textual conflict. Integration failures reconciled with required automatic upgrade: exact original-layer archives, untouched outer fields/order/source bytes and exact native save/reopen are asserted. Guard archive/diagnostic-bearing layers against unsupported old readers (N3/N4); raw legacy 3D alone still needs no marker. | `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-scene3d -p pm-kernel --no-fail-fast`: 356 passed, 0 failed, 1 ignored |
| 3d-render | `55f23d2a323724b392f0e8f2dd5f76acb2a239db` | gpu.rs: retain both Denoiser/DenoiseFn public aliases and fallible guided/OIDN paths (N1/N3). Optional OIDN 2.5.1 suite: 31 passed (same 28 plus 3), using verified upstream SHA-256. Native engine-service test BLOCKED by Apple-only objc2, explicitly not passed. Vulkan llvmpipe is not Metal proof. | `cargo test --locked --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host`: 28 passed, 0 failed, 0 ignored |
