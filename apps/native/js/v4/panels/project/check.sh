#!/bin/sh
set -eu
project_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
cd "$project_dir/../../../.."
cargo test --locked -p pm-panel-media -p pm-surface
cargo clippy --locked -p pm-panel-media --all-targets -- -D warnings
cd "$project_dir/../.."
cargo test --locked --manifest-path panels/project/native/Cargo.toml
cargo clippy --locked --manifest-path panels/project/native/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path panels/project/native/Cargo.toml --check
bun install --frozen-lockfile
bun test
bun run typecheck
bunx tsc --noEmit -p panels/project/tsconfig.json
bunx prettier --check 'panels/project/*.{ts,tsx,json,md}' 'panels/project/tests/*.{ts,tsx}'
