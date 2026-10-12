#!/bin/sh
# S4 layout half, end to end: Canvas2D capture in hidden Electron, native
# comparison, registry replay, subsets, and the WASM identity check under
# JavaScriptCore (Bun) and V8 (Electron's Node).
# Usage: engine/spikes/s4-text/scripts/run-layout.sh [--skip-capture]
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../../.." && pwd)
. "$repo/engine/env.sh"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cd "$here"
if [ "${1:-}" != "--skip-capture" ]; then
  bun capture/run.ts measure
fi
nice -n 15 cargo build --release -p s4-compare
nice -n 15 cargo build --release -p s4-wasm --target wasm32-unknown-unknown
./target/release/s4-compare
./target/release/s4-compare --replay
./target/release/s4-compare --replay --subsets
bun scripts/wasm-identity.mjs
electron="$repo/apps/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"
ELECTRON_RUN_AS_NODE=1 "$electron" scripts/wasm-identity.mjs
