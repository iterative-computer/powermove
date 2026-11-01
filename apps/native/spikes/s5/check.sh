#!/bin/zsh
# The enclosing workspace currently contains an unmatched crates/* member.
# Build the exact S5 sources with an isolated manifest, using the native pin/cache.
set -eu
native_root="$(cd "$(dirname "$0")/../.." && pwd)"
source "$native_root/../../engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$native_root"
python3 - "$native_root/spikes/s5" "${@:-build}" <<'PY'
import os, pathlib, shutil, subprocess, sys, tempfile, time, signal
root = pathlib.Path(sys.argv[1])
if shutil.disk_usage('/').free < 4 * 1024**3:
    sys.exit('S5 STOP: less than 4 GiB free')
subprocess.run(['rustc', '-V'], check=True)
with tempfile.TemporaryDirectory(prefix='pm-s5-manifest-') as tmp:
    manifest = pathlib.Path(tmp) / 'Cargo.toml'
    manifest.write_text(root.joinpath('Cargo.toml').read_text() + '\n[workspace]\n[[bin]]\nname="pm-native-s5-spike"\npath=' + repr(str(root / 'src/main.rs')).replace("'", '"') + '\n')
    if root.joinpath('Cargo.lock').exists(): shutil.copy(root / 'Cargo.lock', pathlib.Path(tmp) / 'Cargo.lock')
    cmd = ['cargo', sys.argv[2], '--manifest-path', str(manifest), '-j', '4']
    proc = subprocess.Popen(cmd, start_new_session=True)
    while proc.poll() is None:
        if shutil.disk_usage('/').free < 4 * 1024**3:
            os.killpg(proc.pid, signal.SIGTERM)
            sys.exit('S5 STOP: less than 4 GiB free during build')
        time.sleep(1)
    if proc.returncode: sys.exit(proc.returncode)
    shutil.copy(pathlib.Path(tmp) / 'Cargo.lock', root / 'Cargo.lock')
PY
