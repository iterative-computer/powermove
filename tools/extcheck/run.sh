#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
mkdir -p /tmp/pm-extcheck
if [[ $(/tmp/pm-shots/is_locked.sh) != unlocked ]]; then
  echo 'Screen is locked; on-screen extension capture cannot run.' >&2
  exit 2
fi
# Each reference capture acquires gui.lock through gui_capture.py. V8 work needs no screen.
exec python3 tools/extcheck/run.py "$@"
