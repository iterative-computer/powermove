#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
while [[ -e /tmp/pm-shots/PAUSE_LOCAL ]]; do
  echo 'Local execution paused by owner; checking again in five minutes'
  sleep 300
done
source engine/env.sh
if [[ $(uname -s) == Darwin ]]; then
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/Developer/pm-target}"
  export RUSTY_V8_ARCHIVE="${PM_S1_ARTIFACT:-/private/tmp/pm-s1-v8-artifact-final}/librusty_v8.a"
  export RUSTY_V8_SRC_BINDING_PATH="${PM_S1_ARTIFACT:-/private/tmp/pm-s1-v8-artifact-final}/src_binding.rs"
  (cd "$(dirname "$RUSTY_V8_ARCHIVE")" && shasum -a 256 -c SHA256SUMS)
else
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/work/target}"
  # Linux uses upstream's prebuilt archive, never the macOS sandbox artifact.
  unset RUSTY_V8_ARCHIVE RUSTY_V8_SRC_BINDING_PATH
fi
export CARGO_BUILD_JOBS=4
export RUST_TEST_THREADS=4
# Match the workspace's debug=0 test profile and avoid a second large dependency build.
export CARGO_PROFILE_DEV_DEBUG=0
unset V8_FROM_SOURCE RUSTY_V8_SKIP_DOWNLOAD DOCS_RS
python3 - "${1:-test}" "${2:-pm-compat-runner}" "${3:-}" <<'PY'
import os, signal, subprocess, sys
macos = sys.platform == "darwin"
sys.path.insert(0,"tools/extcheck")
from resources import disk_ready
mode=sys.argv[1]
package=sys.argv[2]
example=sys.argv[3]
assert mode in ('build','test','clippy','fmt'), 'choose build, test, clippy or fmt'
assert package in ('pm-compat-runner','pm-compat-host','pm-store-host','pm-runtime-v8'), 'unknown adapter package'
assert not example or (mode=='build' and package=='pm-compat-host' and example=='remote-dom-smoke'), 'unknown example'
import time
from pathlib import Path
pause_file=Path('/tmp/pm-shots/PAUSE_LOCAL')
def stop_child(child):
 try:os.killpg(child.pid,signal.SIGTERM)
 except ProcessLookupError:pass
 try:child.wait(timeout=5)
 except subprocess.TimeoutExpired:
  try:os.killpg(child.pid,signal.SIGKILL)
  except ProcessLookupError:pass
  child.wait()
args=['nice','-n','20','flock','/tmp/pm-build.lock','cargo','+1.96.1',mode,'--manifest-path','apps/native/Cargo.toml','-p',package]
if example:
 assert macos, 'WebKit probes require macOS; Linux runs the portable host tests'
 args+=['--example',example,'--features','macos-probes']
if mode=='fmt':args+=['--','--check']
else:
 args+=['--locked']
 if macos and package in ('pm-compat-runner','pm-runtime-v8'):args+=['--features','macos-sandbox']
 if mode=='clippy':args+=['--all-targets','--no-deps','--','-D','warnings']
commands=[args]
if mode=='test' and package=='pm-compat-runner':
 # The integration test launches the real confined helper beside the test binary.
 commands.insert(0,['nice','-n','20','flock','/tmp/pm-build.lock','cargo','+1.96.1','build',
                  '--manifest-path','apps/native/Cargo.toml','-p','pm-runtime-v8','--bin','pm-store-helper',
                  '--locked'] + (['--features','macos-sandbox'] if macos else []))
while commands:
 while True:
  if pause_file.exists():
   print('Local execution paused by owner; checking again in five minutes',flush=True)
   time.sleep(300)
  elif not disk_ready():
   print('Waiting for APFS container free space at or above 5 GB',flush=True)
   time.sleep(120)
  else:break
 child=subprocess.Popen(commands[0],start_new_session=True)
 paused=False
 try:
  while child.poll() is None:
   if pause_file.exists():
    stop_child(child)
    print('Build paused by owner',flush=True)
    paused=True
    break
   if not disk_ready():
    stop_child(child)
    print('Build paused below 5 GB; will retry at or above 5 GB',flush=True)
    paused=True
    break
   try:child.wait(timeout=.25)
   except subprocess.TimeoutExpired:pass
 finally:
  if child.poll() is None:stop_child(child)
 if not paused:
  if child.returncode:raise SystemExit(child.returncode)
  commands.pop(0)
PY
