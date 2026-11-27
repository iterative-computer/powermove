#!/bin/sh
set -eu
# pm-agents is integrated in the shared lockfile; all checks use that lock.
cd "$(dirname "$0")/../../.."
cargo test --locked -p pm-agents
cargo clippy --locked -p pm-agents --all-targets --no-deps -- -D warnings
cargo fmt -p pm-agents --check
