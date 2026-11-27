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
    run_guarded cargo build -p pm-compat-host --features macos-probes --examples
    ;;
  test)
    run_guarded cargo test -p pm-compat-host --lib
    run_guarded cargo build -p pm-compat-host --features macos-probes --example offscreen-probe
    page="$(mktemp -d)"
    (cd js/compat && run_guarded bun scripts/build-app.ts "$page")
    run_guarded "$CARGO_TARGET_DIR/debug/examples/offscreen-probe" "$page/page.js"
    ;;
  *) echo 'Usage: check.sh [test|build]' >&2; exit 2 ;;
esac
