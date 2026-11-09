#!/bin/sh
# pm-text text gate, end to end: unit tests, the Canvas2D corpus regression
# test (native, replay, subsets), and native vs WASM identity under
# JavaScriptCore (Bun) and V8 (Electron's Node, from a v1 checkout with its
# dependencies installed: PM_V1_REF, default /tmp/pm-ref-v1.2.0).
# Usage: engine/crates/pm-text/scripts/check.sh
set -eu
crate=$(cd "$(dirname "$0")/.." && pwd)
engine=$(cd "$crate/../.." && pwd)
electron="${PM_V1_REF:-/tmp/pm-ref-v1.2.0}/apps/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"
if [ ! -x "$electron" ]; then
  echo "check.sh: the V8 identity stage needs $electron; point PM_V1_REF at a v1 checkout with its dependencies installed" >&2
  exit 1
fi
. "$engine/env.sh"
cd "$engine"
target="${CARGO_TARGET_DIR:-$engine/target}"
out="$target/pm-text/wasm"
nice -n 15 cargo test --release -p pm-text -- --test-threads=4
nice -n 15 cargo build --release -p pm-text --no-default-features --features corpus-tools --bin pm-text-corpus
nice -n 15 cargo build --release -p pm-text --example layout_wasm --target wasm32-unknown-unknown --no-default-features --features level2
"$target/release/pm-text-corpus" export --out "$out" 2>/dev/null
wasm="$target/wasm32-unknown-unknown/release/examples/layout_wasm.wasm"
bun "$crate/scripts/wasm-identity.mjs" "$out" "$wasm"
ELECTRON_RUN_AS_NODE=1 "$electron" "$crate/scripts/wasm-identity.mjs" "$out" "$wasm"
