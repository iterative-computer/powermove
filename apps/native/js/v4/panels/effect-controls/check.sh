#!/bin/sh
set -eu
repo=$(CDPATH= cd "$(dirname "$0")/../../../../../.." && pwd -P)
cd "$repo/apps/native"
cargo test --locked -p pm-panel-inspector -p pm-effect-controls-wire -p pm-kernel
cargo clippy --locked -p pm-panel-inspector -p pm-effect-controls-wire --all-targets --no-deps -- -D warnings
cargo fmt -p pm-panel-inspector -p pm-effect-controls-wire --check
cargo build --locked -p pm-effect-controls-wire --bins
cd js/v4
bun install --frozen-lockfile
bunx tsc --noEmit -p panels/effect-controls/tsconfig.json
bunx prettier --check panels/effect-controls/*.ts panels/effect-controls/*.tsx panels/effect-controls/test/*.ts panels/effect-controls/test/*.tsx panels/effect-controls/tsconfig.json
bun test
