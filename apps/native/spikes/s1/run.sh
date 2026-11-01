#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
: "${PM_S1_ARTIFACT:?Download the CI artifact first}"
./check.sh build
bin="$HOME/Developer/pm-target/release/pm-native-s1-spike"
# Hardened runtime + only allow-jit: the spike is statically linked.
codesign --force --sign - --options runtime --entitlements jit-entitlements.plist "$bin"
codesign --verify --strict "$bin"
codesign --display --entitlements :- "$bin"
"$bin" --all
"$bin" --effects
"$bin" --js "$PWD/jit.js"
npm ci --ignore-scripts --no-audit --no-fund
npm run bundle
for fixture in preact react compiler; do
  "$bin" --js "$PWD/.generated/$fixture.js"
done
