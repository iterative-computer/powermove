#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
# Match the workspace's debug=0 test profile and avoid a second large dependency build.
export CARGO_PROFILE_DEV_DEBUG=0
export RUSTY_V8_ARCHIVE="${PM_S1_ARTIFACT:-/private/tmp/pm-s1-v8-artifact-final}/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="${PM_S1_ARTIFACT:-/private/tmp/pm-s1-v8-artifact-final}/src_binding.rs"
unset V8_FROM_SOURCE RUSTY_V8_SKIP_DOWNLOAD DOCS_RS
(cd "$(dirname "$RUSTY_V8_ARCHIVE")" && shasum -a 256 -c SHA256SUMS)
python3 - "${1:-test}" <<'PY'
import os, shutil, signal, subprocess, sys
mode=sys.argv[1]
assert mode in ('build','test','clippy','fmt'), 'choose build, test, clippy or fmt'
assert shutil.disk_usage('/').free >= 8*1024**3, 'STOP: less than 8 GiB free'
args=['flock','/tmp/pm-build.lock','cargo','+1.96.1',mode,'--manifest-path','apps/native/Cargo.toml','-p','pm-compat-runner']
if mode=='fmt':args+=['--','--check']
else:
 args+=['--locked','--features','macos-sandbox']
 if mode=='clippy':args+=['--all-targets','--no-deps','--','-D','warnings']
child=subprocess.Popen(args,start_new_session=True)
while child.poll() is None:
 if shutil.disk_usage('/').free < 8*1024**3:
  os.killpg(child.pid,signal.SIGTERM)
  child.wait()
  raise SystemExit('STOP: less than 8 GiB free during build')
 try:child.wait(timeout=.25)
 except subprocess.TimeoutExpired:pass
raise SystemExit(child.returncode)
PY
