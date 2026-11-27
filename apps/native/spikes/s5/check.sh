#!/bin/zsh
set -eu
native_root="$(cd "$(dirname "$0")/../.." && pwd)"
source "$native_root/../../engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd "$native_root"
df -h /
python3 - "${1:-build}" <<'PY'
import os, pathlib, shutil, signal, subprocess, sys, tempfile, time
if shutil.disk_usage('/').free < 4 * 1024**3:
    sys.exit('S5 STOP: less than 4 GiB free')
subprocess.run(['rustc', '-V'], check=True)
# --locked prevents this lane from changing the shared workspace lockfile.
args = ['cargo', sys.argv[1], '--locked', '-p', 'pm-native-s5-spike', '-j', '4']
if os.environ.get('S5_ISOLATED') == '1':
    # Exact owned sources, no shared lock edits. Reuse the only authorized target.
    scratch_dir = tempfile.TemporaryDirectory(prefix='s5b-manifest-')
    scratch = pathlib.Path(scratch_dir.name)
    source = pathlib.Path('spikes/s5').resolve()
    manifest = (source / 'Cargo.toml').read_text()
    manifest += '\n[workspace]\n[[bin]]\nname = "pm-native-s5-spike"\npath = ' + repr(str(source / 'src/main.rs')) + '\n'
    (scratch / 'Cargo.toml').write_text(manifest)
    args = ['cargo', sys.argv[1], '--offline', '--manifest-path', str(scratch / 'Cargo.toml'), '-j', '4']
proc = subprocess.Popen(args, start_new_session=True)
while proc.poll() is None:
    if shutil.disk_usage('/').free < 4 * 1024**3:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait()
        sys.exit('S5 STOP: less than 4 GiB free during build')
    time.sleep(1)
sys.exit(proc.returncode)
PY
