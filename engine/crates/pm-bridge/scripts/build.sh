#!/bin/sh
# Builds the pm-bridge addon (the font registry and the frame bridge) and installs it as
# engine/target/pm-bridge/pm_bridge.node, where Electron main looks for it in development
# (apps/desktop/src/main/engine-fonts.ts, apps/desktop/src/main/engine/index.ts).
# --lib: the rendezvous test peer binary links without Node's symbols.
# Usage: engine/crates/pm-bridge/scripts/build.sh
set -eu
engine=$(cd "$(dirname "$0")/../../.." && pwd)
. "$engine/env.sh"
cd "$engine"
nice -n 15 cargo build --release -p pm-bridge --lib --features node
mkdir -p "$engine/target/pm-bridge"
cp "$engine/target/release/libpm_bridge.dylib" "$engine/target/pm-bridge/pm_bridge.node"
echo "$engine/target/pm-bridge/pm_bridge.node"
