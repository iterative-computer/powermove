#!/bin/bash
# Build and sign private diagnostic copies; never modify a shipping app bundle.
set -euo pipefail
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'BLOCKED: signed seatbelt/JIT verification requires macOS.' >&2
  exit 2
fi
if [[ "$(uname -m)" != arm64 ]]; then
  echo 'BLOCKED: this verified S1 archive is ARM64; an independently verified Intel pair is required.' >&2
  exit 2
fi
: "${RUSTY_V8_ARCHIVE:?Set the verified S1 sandbox archive path}"
: "${RUSTY_V8_SRC_BINDING_PATH:?Set its matching generated binding path}"
if [[ -n "${V8_FROM_SOURCE+x}" ]]; then
  echo 'Refusing a V8 source build; use the verified S1 artifact pair.' >&2
  exit 2
fi
PM_RUNTIME_CRATE="$(cd "$(dirname "$0")/.." && pwd -P)"
PM_RUNTIME_VERIFY="$(mktemp -d "${TMPDIR:-/tmp}/pm-runtime-signed.XXXXXX")"
PM_RUNTIME_VERIFY="$(cd "$PM_RUNTIME_VERIFY" && pwd -P)"
export PM_RUNTIME_VERIFY
echo "Diagnostic copies and evidence: $PM_RUNTIME_VERIFY"
python3 - <<'PY'
import hashlib
import os

expected = {
    'RUSTY_V8_ARCHIVE': '0cc50754a6bb75fa5fbf3d8e1742723a745ab47e2de260711a77ac2aa1519ea9',
    'RUSTY_V8_SRC_BINDING_PATH': '6da40ff4a76e455222ddf2a902ab60d2e0d69a76b652d90267de4a33e47ff31f',
}
for variable, digest in expected.items():
    h = hashlib.sha256()
    with open(os.environ[variable], 'rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            h.update(block)
    if h.hexdigest() != digest:
        raise SystemExit('S1 artifact identity mismatch: ' + variable)
PY
cargo +1.96.1 test --locked --release --manifest-path "$PM_RUNTIME_CRATE/isolated/Cargo.toml" \
  --features macos-sandbox --no-run --message-format=json \
  > "$PM_RUNTIME_VERIFY/artifacts.jsonl"
python3 - <<'PY'
import json
import os
import shutil

root = os.environ['PM_RUNTIME_VERIFY']
found = {}
with open(os.path.join(root, 'artifacts.jsonl')) as source:
    for line in source:
        artifact = json.loads(line)
        if artifact.get('reason') != 'compiler-artifact' or not artifact.get('executable'):
            continue
        name = artifact['target']['name']
        if name == 'store_helper' or (name == 'pm-store-helper' and not artifact['profile']['test']):
            found[name] = artifact['executable']
for name in ('store_helper', 'pm-store-helper'):
    if name not in found:
        raise SystemExit('Cargo did not report required executable: ' + name)
    shutil.copy2(found[name], os.path.join(root, name))
PY
for PM_RUNTIME_EXE in "$PM_RUNTIME_VERIFY/pm-store-helper" "$PM_RUNTIME_VERIFY/store_helper"; do
  codesign --force --sign "${PM_RUNTIME_SIGN_IDENTITY:--}" --options runtime \
    --entitlements "$PM_RUNTIME_CRATE/scripts/jit.entitlements.plist" "$PM_RUNTIME_EXE"
  codesign --verify --strict --verbose=2 "$PM_RUNTIME_EXE"
  codesign --display --verbose=4 "$PM_RUNTIME_EXE" 2> "$PM_RUNTIME_EXE.signature.txt"
  codesign --display --entitlements :- "$PM_RUNTIME_EXE" > "$PM_RUNTIME_EXE.entitlements.plist"
done
python3 - <<'PY'
import os
import plistlib
import re

root = os.environ['PM_RUNTIME_VERIFY']
for name in ('pm-store-helper', 'store_helper'):
    with open(os.path.join(root, name + '.entitlements.plist'), 'rb') as source:
        if plistlib.load(source) != {'com.apple.security.cs.allow-jit': True}:
            raise SystemExit('Unexpected or missing signing entitlement: ' + name)
    with open(os.path.join(root, name + '.signature.txt')) as source:
        flags = re.search(r'^CodeDirectory\b.*\bflags=0x([0-9a-fA-F]+)\b', source.read(), re.MULTILINE)
        if flags is None or not int(flags.group(1), 16) & 0x10000:
            raise SystemExit('Hardened runtime flag missing: ' + name)
PY
PM_STORE_HELPER="$PM_RUNTIME_VERIFY/pm-store-helper" \
  "$PM_RUNTIME_VERIFY/store_helper" --test-threads=1 --nocapture \
  > "$PM_RUNTIME_VERIFY/probes.log" 2>&1
cat "$PM_RUNTIME_VERIFY/probes.log"
echo 'Signed diagnostic probes passed. Shipping-bundle/notarization and supported-OS verification remain required.'
