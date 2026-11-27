#!/bin/zsh
# Reproduction entry point: one pinned toolchain and one shared target.
set -eu
repo_root="$(cd "$(dirname "$0")/../../../../.." && pwd)"
source "$repo_root/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$repo_root/apps/native"
exec python3 spikes/s2/scripts/check-ptext-120.py "$@"
