#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../../../.."
repo=$(git rev-parse --show-toplevel)
source "$repo/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
python3 - <<'PY'
import shutil
assert shutil.disk_usage('/').free >= 4 * 1024**3, 'STOP: less than 4 GiB free'
PY
if [[ ${V8_FROM_SOURCE:-0} != 0 ]]; then
  echo 'V8 source builds are CI-only. Unset V8_FROM_SOURCE.' >&2; exit 1
fi
: "${PM_S1_ARTIFACT:?Set PM_S1_ARTIFACT to the downloaded CI artifact directory}"
(cd "$PM_S1_ARTIFACT" && shasum -a 256 -c SHA256SUMS)
export RUSTY_V8_ARCHIVE="$PM_S1_ARTIFACT/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="$PM_S1_ARTIFACT/src_binding.rs"
export RUSTY_V8_ARCHIVE_SHA256=$(shasum -a 256 "$RUSTY_V8_ARCHIVE" | cut -d ' ' -f1)
unset RUSTY_V8_SKIP_DOWNLOAD DOCS_RS
cargo +1.96.1 "${1:-build}" --locked --release --manifest-path "$repo/apps/native/spikes/s1/isolated/Cargo.toml"
