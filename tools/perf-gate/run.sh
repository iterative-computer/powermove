#!/bin/zsh
# Own only this disposable reference and the explicitly assigned release target.
set -eu
set -o pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
reference=/tmp/pm-ref-v1.2.0
target="$HOME/Developer/pm-target"
output="$(mktemp -d /tmp/pm-perf-gate-XXXXXX)"
created=0
built=0
cleanup() {
  if (( created )); then
    # --force removes only our untracked helper, ignored build output and node_modules.
    git -C "$repo" worktree remove --force "$reference" || print -u2 'Reference cleanup failed; inspect the owned temporary worktree.'
  fi
  if (( built )); then
    python3 - "$target/release" <<'PY'
from pathlib import Path
import shutil, sys
p = Path(sys.argv[1])
assert p == Path.home() / 'Developer/pm-target/release' and not p.is_symlink()
if p.exists(): shutil.rmtree(p)
PY
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cd "$repo"
if [[ -e "$reference" || -e "$target/release" ]]; then
  print -u2 'Refusing to reuse or remove an existing reference/release build owned by another run.'
  exit 1
fi
disk_check() {
  df -h /
  python3 -c 'import shutil; assert shutil.disk_usage("/").free >= 8*1024**3, "Stop: less than 8 GiB free"'
}
disk_check
git worktree add --detach "$reference" v1.2.0
created=1
cd "$reference"
bun install --frozen-lockfile
disk_check
bun run --cwd apps/desktop build
cd "$repo"
source engine/env.sh
export CARGO_TARGET_DIR="$target"
disk_check
built=1
cargo +1.96.1 build --manifest-path apps/native/Cargo.toml --release -p pm-native-app
bun tools/perf-gate/prepare.ts "$output/corpus" "$reference"
print "Raw results (outside Git): $output/results.json"
python3 tools/perf-gate/driver.py --corpus "$output/corpus" --output "$output/results.json" "$@"
