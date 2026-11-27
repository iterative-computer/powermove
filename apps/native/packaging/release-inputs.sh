#!/bin/bash
# Fetches the pinned public inputs a release bundles into a new folder and prints
# them as NAME=value lines (append to $GITHUB_ENV in CI, or `export` locally):
#   SPARKLE_DIR             Sparkle 2.10.1 (sparkle-pin.json), digest-checked, unpacked
#   PM_CODEX_VENDOR         Codex native vendor tree (runtime-pins.json)
#   PM_CLAUDE_RUNTIME       Claude Code native binary (runtime-pins.json)
#   PM_TRANSCRIBE_RUNTIME   transcribe.cpp contract.json + dylibs (release-inputs.json)
# Downloads only public archives; never signs, installs or runs what it fetches.
#   apps/native/packaging/release-inputs.sh <new-folder> [sparkle]   (sparkle: only SPARKLE_DIR)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
native=$(dirname "$here")
out=${1:?new folder}
[ -e "$out" ] && { echo "release-inputs: $out exists; choose a new folder" >&2; exit 1; }
mkdir -p "$out"
out=$(cd "$out" && pwd)

# npm <name> <spec> <sri>: pack the exact version, check its SHA-512, unpack.
npm_input() {
  local spec=$2 sri=$3 dir="$out/$1"
  mkdir -p "$dir"
  local tarball
  tarball=$(cd "$dir" && npm pack --silent --ignore-scripts "$spec" | tail -1)
  local actual
  actual="sha512-$(openssl dgst -sha512 -binary "$dir/$tarball" | base64)"
  [ "$actual" = "$sri" ] || { echo "release-inputs: $spec does not match its pinned digest" >&2; exit 1; }
  tar -xzf "$dir/$tarball" -C "$dir"
}
pin() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1]))
for k in sys.argv[2].split("."): d=d[k] if isinstance(d,dict) else next(p for p in d if p["package"]==k)
print(d)' "$@"; }

sparkle() {
  # Sparkle: the reviewed upstream archive, digest first, then unpacked privately.
  curl -fsSL --proto =https -o "$out/Sparkle.tar.xz" "$(pin "$here/sparkle-pin.json" archiveURL)"
  python3 "$here/package.py" verify-sparkle-archive "$out/Sparkle.tar.xz"
  mkdir "$out/sparkle"
  tar -xJf "$out/Sparkle.tar.xz" -C "$out/sparkle"
  echo "SPARKLE_DIR=$out/sparkle"
}
if [ "${2:-}" = sparkle ]; then sparkle; exit 0; fi

pins="$native/crates/agents/fixtures/runtime-pins.json"
codex=$(pin "$pins" codex)
claude=$(pin "$pins" claude)
npm_input codex "@openai/codex@$codex-darwin-arm64" "$(pin "$pins" pins.@openai/codex-darwin-arm64.integrity)"
npm_input claude "@anthropic-ai/claude-code-darwin-arm64@$claude" \
  "$(pin "$pins" pins.@anthropic-ai/claude-code-darwin-arm64.integrity)"
inputs="$here/release-inputs.json"
npm_input transcribe "$(pin "$inputs" transcribe.spec)" "$(pin "$inputs" transcribe.integrity)"
contract=$(find "$out/transcribe/package" -name contract.json -print -quit)
[ -n "$contract" ] || { echo "release-inputs: transcribe runtime has no contract.json" >&2; exit 1; }

sparkle
echo "PM_CODEX_VENDOR=$out/codex/package/vendor/aarch64-apple-darwin"
echo "PM_CLAUDE_RUNTIME=$out/claude/package/claude"
echo "PM_TRANSCRIBE_RUNTIME=$(dirname "$contract")"
