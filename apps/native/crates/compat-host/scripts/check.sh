#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../../.." && pwd)"
source "$ROOT/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$ROOT/apps/native"
# Build and test only this lane; never clean the shared target or change caches.
# GUARD_SECONDS bounds the owned child (default 8 minutes for builds/tests).
run_guarded() {
  GUARD_SECONDS="${GUARD_SECONDS:-480}" python3 - "$@" <<'PY'
import os, shutil, subprocess, sys, time
minimum = 6 * 1024**3
limit = int(os.environ['GUARD_SECONDS'])
if shutil.disk_usage('/').free < minimum:
    raise SystemExit('compat_disk_below_6_gib: stopped before launch')
p = subprocess.Popen(sys.argv[1:])
start = time.monotonic()
while p.poll() is None:
    if shutil.disk_usage('/').free < minimum or time.monotonic() - start > limit:
        p.terminate()
        try: p.wait(timeout=5)
        except subprocess.TimeoutExpired: p.kill(); p.wait()
        raise SystemExit(f'compat_disk_or_{limit}_second_limit: owned child stopped')
    time.sleep(1)
raise SystemExit(p.returncode)
PY
}
case "${1:-test}" in
  build)
    run_guarded cargo build -p pm-compat-host --features smoke,macos-probes --examples
    ;;
  test)
    run_guarded cargo test -p pm-compat-host --lib
    run_guarded cargo build -p pm-compat-host --features macos-probes --example offscreen-probe
    run_guarded "$CARGO_TARGET_DIR/debug/examples/offscreen-probe"
    ;;
  smoke)
    run_guarded cargo build -p pm-compat-host --features smoke --example shell-smoke
    # A locked session is never reported as an on-screen pass.
    swift -e 'import CoreGraphics; if let d = CGSessionCopyCurrentDictionary() as? [String:Any], (d["CGSSessionScreenIsLocked"] as? Bool) == true { print("compat_screen_locked: smoke UNVERIFIED"); exit(2) }'
    mkdir -p /tmp/pm-shots
    lock=/tmp/pm-shots/gui.lock
    while ! (set -o noclobber; echo compathost > "$lock") 2>/dev/null; do
      echo 'GUI lock held; retrying in 30 seconds.'
      sleep 30
    done
    release_gui() {
      if [[ -f "$lock" ]] && [[ "$(cat "$lock")" == compathost ]]; then rm "$lock"; fi
      true
    }
    trap release_gui EXIT INT TERM
    # The smoke bounds itself to 35 s and a watchdog thread to 45 s; this outer
    # guard stops a process that is wedged beyond both.
    GUARD_SECONDS=90 run_guarded "$CARGO_TARGET_DIR/debug/examples/shell-smoke"
    ;;
  *) echo 'Usage: check.sh [test|build|smoke]' >&2; exit 2 ;;
esac
