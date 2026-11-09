#!/usr/bin/env bash
# Synthetic data only. Returns the acceptance verdict, including known failures.
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1
if [[ -f /workspace/.tools/activate.sh ]]; then source /workspace/.tools/activate.sh; fi
acceptance_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../../.." && pwd)"
cd "$acceptance_root"
acceptance_work="$acceptance_root/work/native-acceptance"
mkdir -p "$acceptance_work"
acceptance_runtime_features=()
if [[ "$(uname -s)" == Darwin ]]; then acceptance_runtime_features=(--features pm-acceptance-runtime/macos-sandbox); fi
acceptance_packages=(-p pm-acceptance -p pm-acceptance-runtime "${acceptance_runtime_features[@]}")
python3 -m unittest discover -s apps/native/crates/acceptance/scripts -p 'test_*.py'
python3 apps/native/crates/acceptance/src/e2e/annotate.py
cargo test --locked --manifest-path apps/native/Cargo.toml "${acceptance_packages[@]}"
cargo fmt --manifest-path apps/native/Cargo.toml -p pm-acceptance -p pm-acceptance-runtime -- --check
cargo clippy --locked --manifest-path apps/native/Cargo.toml "${acceptance_packages[@]}" --all-targets --no-deps -- -D warnings
cargo build --locked --manifest-path apps/native/Cargo.toml "${acceptance_packages[@]}"
bun build apps/native/crates/acceptance/runtime/compat-probes.ts --target=browser --format=iife --outfile="$acceptance_work/compat-probes.js"
acceptance_runtime_target="${CARGO_TARGET_DIR:-$acceptance_root/apps/native/target}"
set +e
cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance -- \
  --runtime-runner "$acceptance_runtime_target/debug/pm-acceptance-runtime" \
  --compat-bundle "$acceptance_work/compat-probes.js" "$@" --format json > "$acceptance_work/report.json"
acceptance_exit=$?
set -e
if [[ $acceptance_exit -le 2 ]]; then
  python3 - "$acceptance_work/report.json" <<'PY'
import collections, csv, json, pathlib, sys
report = json.load(open(sys.argv[1]))
if report.get('reference'):
    print('Selected reference:', report['reference'])
    legacy = report.get('compatibility')
    if legacy:
        pathlib.Path(sys.argv[1]).with_name('legacy-report.json').write_text(json.dumps(legacy, indent=2)+'\n')
counts = collections.Counter(case['status'] for case in report['cases'])
print(f"{counts['pass']} pass / {counts['fail']} fail / {counts['blocked-by']} blocked-by")
for case in report['cases']:
    if case['status'] == 'fail':
        print(f"FAIL {case['id']}: {case['reason']}")
print(f"Report: {sys.argv[1]}")
blockers = pathlib.Path(sys.argv[1]).with_name('remaining-blockers.csv')
with blockers.open('w', newline='') as output:
    writer = csv.writer(output, lineterminator='\n')
    writer.writerow(['case_id','workflow','kind','needs_macos','blocked_by','reason'])
    for case in report['cases']:
        if case['status'] == 'blocked-by':
            writer.writerow([case['id'],case['workflow'],case['kind'],str(case['needs_macos']).lower(),case['blocked_by'],case['reason']])
print(f"Remaining case blockers: {blockers}")
if report.get('format') == 'powermove.acceptance.main' and report.get('scope') == 'full-inventory':
    import subprocess
    subprocess.run([sys.executable, 'apps/native/crates/acceptance/scripts/census.py', sys.argv[1]], check=True)
PY
fi
exit "$acceptance_exit"
