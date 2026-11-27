#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../../../.."
repo=$(git rev-parse --show-toplevel)
source "$repo/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export MACOSX_DEPLOYMENT_TARGET=14.0
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
python3 - "$repo/apps/native/spikes/s1/isolated/Cargo.toml" "${1:-build}" <<'PYCODE'
import os,shutil,signal,subprocess,sys
proc=subprocess.Popen(['cargo','+1.96.1',sys.argv[2],'--locked','--release','--manifest-path',sys.argv[1]],start_new_session=True)
while True:
    if shutil.disk_usage('/').free < 4*1024**3:
        os.killpg(proc.pid,signal.SIGTERM)
        proc.wait()
        raise SystemExit('STOP: less than 4 GiB free during build')
    try: raise SystemExit(proc.wait(timeout=2))
    except subprocess.TimeoutExpired: pass
PYCODE
