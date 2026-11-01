#!/bin/zsh
set -euo pipefail
cd "${0:A:h}/../../../.."
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
python3 - <<'PYGUARD'
import os
s=os.statvfs('/')
assert s.f_bavail*s.f_frsize >= 4*1024**3, 'S3 STOP: below 4 GiB free'
PYGUARD
bun apps/native/spikes/s3/prepare.ts
mkdir -p /tmp/pm-shots
# This script owns only the lock it created. A failed acquisition never removes it.
until (set -o noclobber; echo s3 > /tmp/pm-shots/gui.lock) 2>/dev/null; do
  sleep 30
done
trap 'rm -f /tmp/pm-shots/gui.lock' EXIT INT TERM
python3 - "$CARGO_TARGET_DIR/debug/pm-native-s3-spike" --document=/tmp/pm-s3-corpus/document.json "$@" <<'PYRUN'
import os,subprocess,sys
s=os.statvfs('/')
assert s.f_bavail*s.f_frsize >= 4*1024**3, 'S3 STOP: below 4 GiB after GUI wait'
# A crashed worker must not strand this lane's GUI lock indefinitely.
sys.exit(subprocess.run(sys.argv[1:], timeout=540).returncode)
PYRUN
