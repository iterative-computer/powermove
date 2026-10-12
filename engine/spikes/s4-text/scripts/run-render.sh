#!/bin/sh
# S4 rendering half, end to end: scenes, Canvas2D capture in hidden Electron,
# the stem-darkening fit, the renderer comparison, and the 4K stress run.
# Usage: engine/spikes/s4-text/scripts/run-render.sh [--skip-fit]
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../../.." && pwd)
. "$repo/engine/env.sh"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cd "$here"
nice -n 15 cargo build --release -p s4-render
./target/release/s4-render scenes
bun capture/run.ts render
if [ "${1:-}" != "--skip-fit" ]; then
  nice -n 10 ./target/release/s4-render fit
fi
nice -n 10 ./target/release/s4-render compare
nice -n 10 ./target/release/s4-render stress
