#!/bin/sh
set -eu
# Lane ownership excludes apps/native/Cargo.lock. Cargo's pinned alternate-lock
# switch lets all checks resolve the real workspace without modifying that file.
cd "$(dirname "$0")/../../.."
export RUSTC_BOOTSTRAP=1
export CARGO_UNSTABLE_LOCKFILE_PATH=true
export CARGO_RESOLVER_LOCKFILE_PATH="$PWD/crates/agents/Cargo.lock"
cargo test --locked -p pm-agents
cargo clippy --locked -p pm-agents --all-targets --no-deps -- -D warnings
cargo fmt -p pm-agents --check
