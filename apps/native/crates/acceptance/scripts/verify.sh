#!/usr/bin/env bash
# Synthetic data only. Returns the acceptance verdict, including known failures.
set -euo pipefail
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
acceptance_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../../.." && pwd)"
cd "$acceptance_root"
acceptance_work="$acceptance_root/work/native-acceptance"
mkdir -p "$acceptance_work"
acceptance_manifest="$acceptance_root/apps/native/crates/acceptance/runtime/Cargo.toml"
acceptance_features=()
if [[ "$(uname -s)" == Darwin ]]; then acceptance_features=(--features macos-sandbox); fi
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance
cargo fmt --manifest-path apps/native/Cargo.toml -p pm-acceptance -- --check
cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance --all-targets --no-deps -- -D warnings
cargo test --locked --manifest-path "$acceptance_manifest" "${acceptance_features[@]}"
cargo fmt --manifest-path "$acceptance_manifest" -- --check
cargo clippy --locked --manifest-path "$acceptance_manifest" "${acceptance_features[@]}" --all-targets --no-deps -- -D warnings
cargo build --locked --manifest-path "$acceptance_manifest" "${acceptance_features[@]}"
bun build apps/native/crates/acceptance/runtime/compat-probes.ts --target=browser --format=iife --outfile="$acceptance_work/compat-probes.js"
# Respect an explicit shared target directory; otherwise use the companion's target.
acceptance_runtime_target="${CARGO_TARGET_DIR:-$acceptance_root/apps/native/crates/acceptance/runtime/target}"
set +e
cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance -- \
  --runtime-runner "$acceptance_runtime_target/debug/pm-acceptance-runtime" \
  --compat-bundle "$acceptance_work/compat-probes.js" "$@" --format json > "$acceptance_work/report.json"
acceptance_exit=$?
set -e
if [[ $acceptance_exit -le 2 ]]; then
  python3 - "$acceptance_work/report.json" <<'PY'
import collections, json, sys
report = json.load(open(sys.argv[1]))
counts = collections.Counter(case['status'] for case in report['cases'])
print(f"{counts['pass']} pass / {counts['fail']} fail / {counts['blocked-by']} blocked-by")
for case in report['cases']:
    if case['status'] == 'fail':
        print(f"FAIL {case['id']}: {case['reason']}")
print(f"Report: {sys.argv[1]}")
PY
fi
exit "$acceptance_exit"
