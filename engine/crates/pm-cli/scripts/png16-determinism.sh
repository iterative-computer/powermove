#!/usr/bin/env bash
# Section 2's determinism bar for lossless outputs (docs/engine/phase3a-gate.md,
# "Determinism: lossless outputs"): `pm-cli render --format png16` of the first
# second of nine corpus projects, twice each into separate folders, then a
# byte comparison of every file. Media come from the capture's project record
# (fixture paths are repository-relative).
#
#   engine/crates/pm-cli/scripts/png16-determinism.sh [capture dir] [out dir]
#
# Run from the repository root after `cargo build --release -p pm-cli`.
set -uo pipefail
corpus="${PM_CORPUS_ROOT:-$HOME/Library/Application Support/Powermove Engine Corpus}"
capture="${1:-$(find "$corpus" -maxdepth 1 -type d -name 'v1-*' | sort | tail -1)}"
out="${2:-$(mktemp -d -t pm-png16)}"
repo="$(cd "$(dirname "$0")/../../../.." && pwd)"
cli="$repo/engine/target/release/pm-cli"
mkdir -p "$out/a" "$out/b"
projects=(3d/primitives 3d/lights 3d/gltf shapes/primitives layers/solid
  backgrounds/radial animation/transforms media/h264-aac-mp4 media/prores422-mov)

frames=0
differ=0
for p in "${projects[@]}"; do
  dir="$capture/projects/$p"
  media=()
  while IFS= read -r arg; do media+=(--media "$arg"); done < <(
    python3 - "$dir/project.json" "$repo" "$capture" <<'PY'
import json, os, sys
record, repo, capture = sys.argv[1:]
for m in json.load(open(record)).get("media", {}).values():
    path = m["source"]["path"]
    if not os.path.isabs(path):
        path = next((p for p in (os.path.join(repo, path), os.path.join(capture, path)) if os.path.exists(p)), path)
    print(f"{m['assetId']}={path}")
PY
  )
  for run in a b; do
    dest="$out/$run/${p//\//_}"
    nice -n 15 "$cli" render "$dir/loaded.json" -o "$dest" --format png16 --range 0:1 --force ${media[@]+"${media[@]}"} \
      >"$out/${p//\//_}-$run.log" 2>&1 || { echo "$p: render $run failed (see $out)"; differ=$((differ + 1)); }
  done
  n=$(find "$out/a/${p//\//_}" -name '*.png' 2>/dev/null | wc -l | tr -d ' ')
  frames=$((frames + n))
  if diff -rq "$out/a/${p//\//_}" "$out/b/${p//\//_}" >/dev/null; then
    echo "$p: $n frames byte-identical"
  else
    echo "$p: DIFFERS"
    differ=$((differ + 1))
  fi
done
echo "png16 determinism: ${#projects[@]} projects, $frames frames per run, $differ differing ($out)"
[ "$differ" -eq 0 ]
