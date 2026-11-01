#!/bin/zsh
set -eu
native_root="$(cd "$(dirname "$0")/.." && pwd)"
repo_root="$(cd "$native_root/../.." && pwd)"
source "$repo_root/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$native_root"
df -h /
python3 scripts/cargo-guard.py build -p pm-native-s2-spike -j 4
python3 scripts/cargo-guard.py test -p pm-native-s2-spike -j 4
"$CARGO_TARGET_DIR/debug/pm-native-s2-spike" --verify
