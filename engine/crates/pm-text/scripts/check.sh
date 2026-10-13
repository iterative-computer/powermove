#!/bin/sh
# pm-text text gate, end to end: unit tests, the Canvas2D corpus regression
# test (native, replay, subsets), and native vs WASM identity under
# JavaScriptCore (Bun) and V8 (Electron's Node).
# Usage: engine/crates/pm-text/scripts/check.sh
set -eu
crate=$(cd "$(dirname "$0")/.." && pwd)
engine=$(cd "$crate/../.." && pwd)
repo=$(cd "$engine/.." && pwd)
. "$engine/env.sh"
cd "$engine"
out="$engine/target/pm-text/wasm"
nice -n 15 cargo test --release -p pm-text -- --test-threads=4
nice -n 15 cargo build --release -p pm-text --no-default-features --features corpus-tools --bin pm-text-corpus
nice -n 15 cargo build --release -p pm-text --example layout_wasm --target wasm32-unknown-unknown --no-default-features --features level2
"$engine/target/release/pm-text-corpus" export --out "$out" 2>/dev/null
wasm="$engine/target/wasm32-unknown-unknown/release/examples/layout_wasm.wasm"
bun "$crate/scripts/wasm-identity.mjs" "$out" "$wasm"
electron="$repo/apps/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"
ELECTRON_RUN_AS_NODE=1 "$electron" "$crate/scripts/wasm-identity.mjs" "$out" "$wasm"
