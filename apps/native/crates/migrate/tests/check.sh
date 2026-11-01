#!/bin/bash
# Build the actual source files without changing the shared workspace lockfile.
set -euo pipefail
PM_S6_CRATE="$(cd "$(dirname "$0")/.." && pwd)"
PM_S6_REPO="$(cd "$PM_S6_CRATE/../../../.." && pwd)"
source "$PM_S6_REPO/engine/env.sh"
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
PM_S6_TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$PM_S6_REPO/apps/native/rust-toolchain.toml")"
pm_s6_check_space() {
  df -h /
python3 - <<'PY'
import shutil
free=shutil.disk_usage('/').free
print('free_bytes='+str(free))
if free < 4*1024**3: raise SystemExit('under 4 GiB free; stopping')
PY
}

PM_S6_BUILD="$(mktemp -d /tmp/pm-s6-check.XXXXXX)"
trap 'rm -rf "$PM_S6_BUILD"' EXIT
python3 - "$PM_S6_CRATE" "$PM_S6_BUILD" <<'PY'
from pathlib import Path
import sys,json
source=Path(sys.argv[1]);out=Path(sys.argv[2]);quote=lambda p:json.dumps(str(p))
s=(source/'Cargo.toml').read_text()+'\n[workspace]\n'
s+='\n[profile.dev]\ndebug = 0\nincremental = false\n[profile.test]\ndebug = 0\nincremental = false\n'
s+='\n[lib]\npath = '+quote(source/'src/lib.rs')+'\n'
for p in sorted((source/'tests').glob('*.rs')):
 s+='\n[[test]]\nname = '+quote(p.stem)+'\npath = '+quote(p)+'\n'
for p in sorted((source/'examples').glob('*.rs')):
 s+='\n[[example]]\nname = '+quote(p.stem)+'\npath = '+quote(p)+'\n'
(out/'Cargo.toml').write_text(s)
PY
if [[ "${1:-}" == "--release-bench" ]]; then
  pm_s6_check_space
  cargo "+$PM_S6_TOOLCHAIN" test --release --manifest-path "$PM_S6_BUILD/Cargo.toml" --test migration large_blob_streaming_throughput -- --nocapture --test-threads=1
else
  cargo "+$PM_S6_TOOLCHAIN" fmt --manifest-path "$PM_S6_BUILD/Cargo.toml" --check
  pm_s6_check_space
  cargo "+$PM_S6_TOOLCHAIN" clippy --manifest-path "$PM_S6_BUILD/Cargo.toml" --all-targets -- -D warnings
  pm_s6_check_space
  cargo "+$PM_S6_TOOLCHAIN" test --manifest-path "$PM_S6_BUILD/Cargo.toml" --tests -- --nocapture
fi
