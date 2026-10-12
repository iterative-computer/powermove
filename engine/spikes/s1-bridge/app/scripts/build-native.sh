#!/bin/sh
# Builds the two napi addons with the pinned toolchain and copies them into app/native.
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
spike="$(cd "$here/.." && pwd)"
. "$spike/../../env.sh"
cd "$spike"
cargo build --release -p s1-bridge
cargo build --release -p s1-engine --features node
mkdir -p "$here/native"
cp target/release/libs1_bridge.dylib "$here/native/s1_bridge.node"
cp target/release/libs1_engine.dylib "$here/native/s1_engine.node"
echo "addons in $here/native"
