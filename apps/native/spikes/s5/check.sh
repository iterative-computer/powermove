#!/bin/zsh
set -eu
native_root="$(cd "$(dirname "$0")/../.." && pwd)"
source "$native_root/../../engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$native_root"
df -h /
python3 - "${1:-build}" <<'PY'
import os, shutil, signal, subprocess, sys, time
if shutil.disk_usage('/').free < 4 * 1024**3:
    sys.exit('S5 STOP: less than 4 GiB free')
subprocess.run(['rustc', '-V'], check=True)
# --locked prevents this lane from changing the shared workspace lockfile.
proc = subprocess.Popen(['cargo', sys.argv[1], '--locked', '-p', 'pm-native-s5-spike', '-j', '4'], start_new_session=True)
while proc.poll() is None:
    if shutil.disk_usage('/').free < 4 * 1024**3:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait()
        sys.exit('S5 STOP: less than 4 GiB free during build')
    time.sleep(1)
sys.exit(proc.returncode)
PY
