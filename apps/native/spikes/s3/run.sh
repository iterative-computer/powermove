#!/bin/zsh
set -euo pipefail
cd "${0:A:h}/../../../.."
source engine/env.sh
export DYLD_LIBRARY_PATH="$PM_FFMPEG_DIR/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
python3 - <<'PYGUARD'
import os
s=os.statvfs('/')
assert s.f_bavail*s.f_frsize >= 6*1024**3, 'S3 STOP: below 6 GiB free'
PYGUARD
bun apps/native/spikes/s3/prepare.ts
mkdir -p /tmp/pm-shots
# Acquisition is bounded and reports the owner; only our successful creator cleans up.
lock=/tmp/pm-shots/gui.lock
started=$SECONDS
until (set -o noclobber; echo engsvc2 > "$lock") 2>/dev/null; do
  echo "S3 waiting for GUI lock (owner: $(cat "$lock" 2>/dev/null || echo unknown)); elapsed=$((SECONDS-started))s" >&2
  if (( SECONDS-started >= ${S3_LOCK_TIMEOUT:-600} )); then
    echo 'S3 GUI acquisition timed out before app launch' >&2; exit 75
  fi
  sleep ${S3_LOCK_RETRY:-30}
done
runner=
cleanup() {
  trap - EXIT INT TERM
  if [[ -n "$runner" ]] && kill -0 "$runner" 2>/dev/null; then
    kill -TERM "$runner" 2>/dev/null || true
    wait "$runner" 2>/dev/null || true
  fi
  rm -f /tmp/pm-shots/gui.lock
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
echo "S3 acquired GUI lock at $(date -u +%FT%TZ)" >&2
binary=pm-native-s3-spike
for arg in "$@"; do
  if [[ "$arg" == --offscreen ]]; then binary=pm-native-s3-headless; fi
done
profile=${S3_PROFILE:-debug}
[[ "$profile" == debug || "$profile" == release ]] || { echo 'S3_PROFILE must be debug or release' >&2; exit 1; }
if [[ "$binary" == pm-native-s3-spike ]]; then
  swift apps/native/spikes/s3/screen-check.swift
fi
python3 - "$CARGO_TARGET_DIR/$profile/$binary" --document=/tmp/pm-s3-corpus/document.json "$@" <<'PYRUN' &
import os,subprocess,sys
s=os.statvfs('/')
assert s.f_bavail*s.f_frsize >= 6*1024**3, 'S3 STOP: below 6 GiB after GUI wait'
# A crashed worker must not strand this lane's GUI lock indefinitely.
env = os.environ.copy()
env["DYLD_LIBRARY_PATH"] = env["PM_FFMPEG_DIR"] + "/lib"
import signal
def terminate(signum, frame):
    raise SystemExit(128 + signum)
signal.signal(signal.SIGTERM, terminate)
print("S3 launching " + sys.argv[1], flush=True)
p = subprocess.Popen(sys.argv[1:], env=env, start_new_session=True)
print(f"S3 pid={p.pid}; run timeout={os.environ.get('S3_RUN_TIMEOUT', '180')}s", flush=True)
try:
    sys.exit(p.wait(timeout=float(os.environ.get('S3_RUN_TIMEOUT', '180'))))
except subprocess.TimeoutExpired:
    print("S3 RUN TIMEOUT: terminating own process group", file=sys.stderr, flush=True)
    os.killpg(p.pid, signal.SIGTERM)
    try: p.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        p.wait()
    sys.exit(124)
finally:
    if p.poll() is None:
        os.killpg(p.pid, signal.SIGTERM)
        try: p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            p.wait()
    print(f"S3 exited {p.returncode}", flush=True)
PYRUN
runner=$!
wait "$runner"
