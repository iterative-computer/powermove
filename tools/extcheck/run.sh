#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
while [[ -e /tmp/pm-shots/PAUSE_LOCAL || -e /tmp/pm-shots/USER_ACTIVE ]]; do
  echo 'GUI capture waiting for owner availability; checking again in five minutes'
  sleep 300
done
mkdir -p /tmp/pm-extcheck
if [[ $(/tmp/pm-shots/is_locked.sh) != unlocked ]]; then
  echo 'Screen is locked; on-screen extension capture cannot run.' >&2
  exit 2
fi
# Each capture role acquires gui.lock through gui_capture.py.
exec python3 tools/extcheck/run.py "$@"
