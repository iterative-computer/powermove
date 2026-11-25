#!/bin/sh
# Powermove notice: Powermove's own tool (design 2.7, POWERMOVE.md "Upstream
# tracking"). Re-pins this tree to another upstream fx commit.
#
#   tools/sync.sh <upstream fx clone> <new commit>             merge, then derive
#   tools/sync.sh --continue <upstream fx clone> <new commit>  after resolving
#
# 1. Reads the current pin from tools/closure.txt and checks that every
#    vendored upstream file without a Powermove notice is upstream's file at
#    that pin, so the notices name all of our changes.
# 2. Exports our patch series (upstream's file at the pin against ours, for
#    every upstream file carrying a notice) to $work/series.diff.
# 3. Merges each patched file three ways (`git merge-file --diff3`: ours, the
#    old pin as base, the new commit) in place, and stops on any conflict,
#    leaving the markers for a person to resolve. Then run with --continue.
# 4. Re-derives the closure at the new commit (tools/derive-closure.sh, which
#    builds it), re-extracts src/libfx_core_runtime.zig from the new N-API
#    entry, and refreshes the verbatim files (LICENSE, notices, fake server).
# 5. Names the upstream build files that changed: ours are trimmed by hand.
set -eu
resume=0
if [ "${1:-}" = "--continue" ]; then resume=1; shift; fi
[ $# -eq 2 ] || { echo "usage: $0 [--continue] <upstream fx clone> <new commit>" >&2; exit 2; }
upstream=$1
new=$(git -C "$1" rev-parse --verify --short=9 "$2^{commit}")
here=$(cd "$(dirname "$0")/.." && pwd)
old=$(sed -n '1s/.*from upstream fx \([0-9a-f]*\):.*/\1/p' "$here/tools/closure.txt")
[ -n "$old" ] || { echo "sync: no pin in tools/closure.txt" >&2; exit 1; }
git -C "$upstream" cat-file -e "$old^{commit}" || { echo "sync: clone lacks the pin $old (git fetch --tags)" >&2; exit 1; }

# Upstream files we changed: a notice, and a file of that path upstream.
patched=""
for file in $(cd "$here" && grep -rl "Powermove notice" src | LC_ALL=C sort); do
  git -C "$upstream" cat-file -e "$old:$file" 2>/dev/null && patched="$patched $file"
done

if [ $resume -eq 0 ]; then
  for file in $(grep -v '^#' "$here/tools/closure.txt"); do
    case " $patched " in *" $file "*) continue ;; esac
    git -C "$upstream" show "$old:$file" | cmp -s - "$here/$file" ||
      { echo "sync: $file differs from upstream $old but carries no Powermove notice" >&2; exit 1; }
  done

  work=$(mktemp -d "${TMPDIR:-/tmp}/fx-sync.XXXXXX")
  for file in $patched; do
    mkdir -p "$work/base/$(dirname "$file")" "$work/ours/$(dirname "$file")"
    git -C "$upstream" show "$old:$file" > "$work/base/$file"
    cp "$here/$file" "$work/ours/$file"
  done
  (cd "$work" && diff -ruN base ours > series.diff) || true
  echo "sync: series of $(echo $patched | wc -w | tr -d ' ') patched files in $work/series.diff"

  conflicts=""
  for file in $patched; do
    if ! git -C "$upstream" cat-file -e "$new:$file" 2>/dev/null; then
      conflicts="$conflicts $file(deleted-upstream)"
      continue
    fi
    git -C "$upstream" show "$new:$file" > "$work/theirs"
    rc=0
    git merge-file --diff3 -L ours -L "fx $old" -L "fx $new" "$here/$file" "$work/base/$file" "$work/theirs" || rc=$?
    [ $rc -eq 0 ] || conflicts="$conflicts $file"
  done
  if [ -n "$conflicts" ]; then
    echo "sync: conflicts (markers left in place):" >&2
    for file in $conflicts; do echo "  $file" >&2; done
    echo "sync: resolve them, then: $0 --continue $upstream $new" >&2
    exit 1
  fi
fi

for file in $patched; do
  if grep -qE '^(<<<<<<<|\|\|\|\|\|\|\||>>>>>>>) ' "$here/$file"; then
    echo "sync: $file still has conflict markers" >&2
    exit 1
  fi
done

"$here/tools/derive-closure.sh" "$upstream" "$new"

work=$(mktemp -d "${TMPDIR:-/tmp}/fx-sync-rt.XXXXXX")
trap 'rm -rf "$work"' EXIT
git -C "$upstream" show "$new:src/napi_core_main.zig" > "$work/napi_core_main.zig"
python3 -I "$here/tools/extract-core-runtime.py" "$work/napi_core_main.zig" "$here/src/libfx_core_runtime.zig"
git -C "$upstream" show "$new:LICENSE" > "$here/LICENSE"
git -C "$upstream" show "$new:THIRD_PARTY_NOTICES.md" > "$here/THIRD_PARTY_NOTICES.md"
git -C "$upstream" show "$new:benchmarks/libfx/fake-inference-server.zig" > "$here/test-support/fake-inference-server.zig"

changed=$(git -C "$upstream" diff --name-only "$old" "$new" -- build.zig build.zig.zon)
[ -z "$changed" ] || echo "sync: upstream changed $(echo $changed): bring what the capi graph needs into ours by hand"
echo "sync: $old -> $new done; update POWERMOVE.md (pin, closure count, patches) and run the Aero gates"
