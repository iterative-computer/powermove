#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
source "$root/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export MACOSX_DEPLOYMENT_TARGET=14.0
[[ ${V8_FROM_SOURCE:-0} == 0 ]] || { echo 'No local V8 source builds'; exit 1; }
export PM_S1_ARTIFACT=${PM_S1_ARTIFACT:-/tmp/pm-s1-v8-artifact}
(cd "$PM_S1_ARTIFACT" && shasum -a 256 -c SHA256SUMS)
export RUSTY_V8_ARCHIVE="$PM_S1_ARTIFACT/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="$PM_S1_ARTIFACT/src_binding.rs"
export RUSTY_V8_ARCHIVE_SHA256=$(shasum -a 256 "$RUSTY_V8_ARCHIVE" | cut -d ' ' -f1)
unset RUSTY_V8_SKIP_DOWNLOAD DOCS_RS
python3 - "$root/apps/native/spikes/s5/s5c/Cargo.toml" "${1:-build}" <<'PY'
import os, shutil, signal, subprocess, sys
assert shutil.disk_usage('/').free >= 6*1024**3, 'STOP under 6 GiB'
profile=[] if os.environ.get('S5C_DEV')=='1' else ['--release']
p=subprocess.Popen(['cargo','+1.96.1',sys.argv[2],'--locked','--offline',*profile,'--manifest-path',sys.argv[1]], start_new_session=True)
while True:
    if shutil.disk_usage('/').free < 6*1024**3:
        os.killpg(p.pid,signal.SIGTERM); p.wait(); sys.exit('STOP under 6 GiB')
    try: sys.exit(p.wait(timeout=2))
    except subprocess.TimeoutExpired: pass
PY
