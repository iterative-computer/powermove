#!/usr/bin/env bash
# Run every S5 measurement: N rounds of Node and the Electron 44 renderer,
# the native vs wasm bit-identity check with its negative control, legacy
# parity, and the summary.
#
#   engine/spikes/s5-wasm/scripts/run-all.sh [rounds]   # default 3
#
# Results stay in results/ (gitignored). Electron windows are hidden and
# never activate.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"
rounds="${1:-3}"
electron="$here/../../../apps/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"
[[ -x "$electron" ]] || { echo "Electron 44 not found at $electron (run bun install in the repository root)" >&2; exit 1; }

./scripts/build.sh
rm -rf results/runs
for i in $(seq 1 "$rounds"); do
  echo "round $i/$rounds: node"
  node --expose-gc js/node-run.mjs > /dev/null
  echo "round $i/$rounds: electron"
  env -u ELECTRON_RUN_AS_NODE "$electron" electron/main.cjs all 2> results/electron-stderr.local.log
  mkdir -p "results/runs/$i"
  cp results/node.local.json results/electron-*.local.json "results/runs/$i/"
done

# node-run.mjs wrote the projects, the times, and the wasm vectors.
status=0
for p in p884 coverage; do
  target/release/s5-native vectors "results/$p.local.json" results/times.local.json "results/vectors-$p.native.local.bin"
  target/system-math/release/s5-native vectors "results/$p.local.json" results/times.local.json "results/vectors-$p.native-system.local.bin"
  echo "$p: native (libm) vs wasm (Node)"
  target/release/s5-native compare "results/vectors-$p.native.local.bin" "results/vectors-$p.wasm-node.local.bin" || status=1
  echo "$p: negative control, native (platform math) vs wasm (expected to differ)"
  if target/release/s5-native compare "results/vectors-$p.native-system.local.bin" "results/vectors-$p.wasm-node.local.bin"; then
    echo "negative control did not differ" >&2
    status=1
  fi
done

bun build js/legacy-parity.mjs --target=node --outfile target/legacy-parity.mjs > /dev/null
node target/legacy-parity.mjs . > /dev/null
node js/summarize.mjs results
exit "$status"
