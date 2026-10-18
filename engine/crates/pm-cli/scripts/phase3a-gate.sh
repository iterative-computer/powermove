#!/usr/bin/env bash
# Phase 3a gate: the engine-side measurements (docs/engine/phase3a-gate.md).
#
#   engine/crates/pm-cli/scripts/phase3a-gate.sh [OUT] [STEP...]
#
# Runs, one after another at low priority, the render parity of the whole
# corpus through `pm-cli parity render` (with the real project and the
# determinism check; again without definitions for the job engine's
# routing), every pm-render parity suite the gate reads (3D,
# Blender, text, shaders, effects, linear blending, antialiasing, motion,
# composite, color), the blur-energy line, native/WASM evaluation identity,
# the 1M-vertex pick measurement, and the engine's share of canvas text
# typing. App rows run separately under the Electron lock
# (docs/engine/phase3a-gate.md, "Reproducing"). Logs and JSON go to OUT (default
# /tmp/phase3a-gate), never into the repository; `status.txt` lists each
# step's exit status. STEP names (the log names below) limit the run.
#
# Machine limits (the engine's shared Mac): nice 15, at most 4 test threads,
# CARGO_BUILD_JOBS from engine/env.sh. Nothing here starts Electron.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
engine="$(cd "$here/../../.." && pwd)"
out="${1:-/tmp/phase3a-gate}"
shift || true
only=("$@")
mkdir -p "$out"
# shellcheck disable=SC1091
source "$engine/env.sh"
cd "$engine" || exit 70
corpus="${PM_CORPUS_ROOT:-$HOME/Library/Application Support/Powermove Engine Corpus}"
newest() { find "$corpus" -maxdepth 1 -type d -name "$1*" | sort | tail -1; }

wanted() {
  [ ${#only[@]} -eq 0 ] && return 0
  local s
  for s in "${only[@]}"; do [ "$s" = "$1" ] && return 0; done
  return 1
}

run() {
  local name=$1
  shift
  wanted "$name" || return 0
  local start
  start=$(date +%s)
  echo "== $name ($(date '+%H:%M:%S'), load $(sysctl -n vm.loadavg))"
  nice -n 15 "$@" >"$out/$name.log" 2>&1
  local status=$?
  echo "$name exit $status $(( $(date +%s) - start ))s load $(sysctl -n vm.loadavg)" | tee -a "$out/status.txt"
}

t() { # a pm-render integration test target, release, 4 threads
  local target=$1
  shift
  run "pm-render-$target" cargo test --release -p pm-render --test "$target" "$@" -- --nocapture --test-threads=4
}

run build cargo build --release -p pm-cli

parity_args=()
for p in v1- render- effects- composite- motion-; do
  c=$(newest "$p")
  [ -n "$c" ] && parity_args+=(--capture "$c")
done
real="$corpus/perf/Powermove ad (perf copy).pmv"
[ -f "$real" ] && parity_args+=(--real "$real")
run parity-render ./target/release/pm-cli parity render "${parity_args[@]}" \
  --determinism --diffs "$out/parity-diffs" --json "$out/parity-render.json"

# What the job engine routes: exports, agent stills, and pm-cli carry no
# definitions yet (docs/engine/effects.md), so effects and transitions route.
run parity-render-jobs ./target/release/pm-cli parity render "${parity_args[@]}" \
  --no-definitions --json "$out/parity-render-jobs.json"

# Render classes the parity runner does not judge itself.
for target in corpus text_corpus text_raster text_level2 text_gpu text_colors \
  scene3d scene3d_gltf scene3d_mesh scene3d_layering scene3d_document scene3d_resources \
  three_programs blender_snapshots blender_composite shader_gate shader_layers \
  effects_webgl effects effects_linear linear color sampling motion retime composite \
  vector stills headless video_end; do
  t "$target"
done

# Section 2's third linear-blending line (a blurred white line conserves energy).
run blur-energy cargo test --release -p pm-cli --test phase3a_bars -- --ignored --nocapture --test-threads=1

# Section 2's determinism bar, second half: native and WASM evaluation are
# bit-identical on the corpus (needs pm_wasm.wasm: bun run --cwd apps/desktop build:wasm).
run eval-wasm ./target/release/pm-cli parity eval --wasm --json "$out/parity-eval.json"

# Phase 3a performance rows the engine measures on its own.
export PM_GLTF_BIG="${PM_GLTF_BIG:-$corpus/big-skinned.glb}"
run pick-1m cargo test --release -p pm-geom3d --test gltf_picks million_vertex_picks_meet_the_targets \
  -- --ignored --nocapture --test-threads=1
run text-typing cargo run --release -p pm-render --example text_typing
echo "logs in $out"
