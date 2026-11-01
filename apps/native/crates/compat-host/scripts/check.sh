#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../../.." && pwd)"
source "$ROOT/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$ROOT/apps/native"
# Build and test only this lane; never clean the shared target or change caches.
run_guarded() {
  python3 - "$@" <<'PY'
import shutil, subprocess, sys, time
minimum = 6 * 1024**3
if shutil.disk_usage('/').free < minimum:
    raise SystemExit('compat_disk_below_6_gib: stopped before launch')
p = subprocess.Popen(sys.argv[1:])
start = time.monotonic()
while p.poll() is None:
    if shutil.disk_usage('/').free < minimum or time.monotonic() - start > 480:
        p.terminate()
        try: p.wait(timeout=5)
        except subprocess.TimeoutExpired: p.kill(); p.wait()
        raise SystemExit('compat_disk_or_8_minute_limit: owned child stopped')
    time.sleep(1)
raise SystemExit(p.returncode)
PY
}
case "${1:-test}" in
  build)
    run_guarded cargo build -p pm-compat-host --features smoke --examples
    ;;
  test)
    run_guarded cargo test -p pm-compat-host --lib
    run_guarded cargo build -p pm-compat-host --example offscreen-probe
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
    run_guarded "$CARGO_TARGET_DIR/debug/examples/shell-smoke"
    ;;
  *) echo 'Usage: check.sh [test|build|smoke]' >&2; exit 2 ;;
esac
