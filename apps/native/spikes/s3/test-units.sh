#!/bin/zsh
set -euo pipefail
cd "${0:A:h}"
source ../../../../engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
python3 - <<'PY'
import os,subprocess
s=os.statvfs('/')
assert s.f_bavail*s.f_frsize >= 4*1024**3, 'S3 STOP: below 4 GiB free'
for name in ['policy','color']:
    binary=os.environ['CARGO_TARGET_DIR']+'/s3-'+name+'-tests'
    subprocess.run(['rustc','--edition=2024','--test','src/'+name+'.rs','-o',binary],check=True)
    subprocess.run([binary],check=True)
PY
cargo fmt --check
