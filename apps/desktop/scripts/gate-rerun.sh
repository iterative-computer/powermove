#!/usr/bin/env bash
# Reruns the Phase 1 and Phase 2 gates (docs/engine/phase1-gate.md,
# docs/engine/phase2-gate.md, "Reproducing"), one step at a time, every
# Electron step under the machine-wide lock, with the 1-minute load average
# recorded before and after each step.
#
#   source engine/env.sh
#   cd apps/desktop && bun install && node scripts/build-engine-addon.mjs \
#     && node ../../engine/crates/pm-wasm/scripts/build.mjs && bun run build
#   scripts/gate-rerun.sh <step> [<step> ...]      (steps below; `list` lists them)
#
# Results go to $GATE_OUT (default ~/Library/Caches/powermove-engine/regate),
# never into the repository: playwright JSON results, native evaluator
# reports, logs, and steps.tsv (step, start, end, load before, load after,
# exit status). Harness summaries go where their scripts put them
# (~/Library/Caches/powermove-engine/measurements/) under the tag $GATE_TAG
# (default regate).
set -u
desktop="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$desktop/../.." && pwd)"
L="$HOME/Library/Caches/powermove-engine/bin/pm-electron-lock"
OUT="${GATE_OUT:-$HOME/Library/Caches/powermove-engine/regate}"
TAG="${GATE_TAG:-regate}"
mkdir -p "$OUT"
cd "$desktop" || exit 2

load1() { sysctl -n vm.loadavg | awk '{print $2}'; }

# The 40 engine-related legacy spec files of the Phase 2 gate, run with
# Use New Engine on for every launch (boot, saves, project files, imports,
# media recovery and replacement, agent media, exports, video playback and
# scrubbing, viewer, GPU recovery, precomps, compositions, 3D, capture).
# The gate's report did not record the file names; this list is
# reconstructed from its categories and failures and has the gate's
# 127 tests at its tip (3d37d8b4).
FLAG_ON_SPECS=(
  boot persistence project-save project-save-large-media project-save-large-history menu-save
  project-file-media project-export-fixture project-close-unchanged
  import-mp4 import-svg import-converted-formats import-prores import-prores-alpha
  import-image-sequence import-animated-image import-progress open-media
  local-media-recovery media-replacement agent-media-handoff agent-media-management
  export-mp4 export-video-frames export-webm
  video-playback-regression video-scrubbing composited-video-sync video-frame-integrity
  playback-audio transparent-sequence-scrubbing gif-preview-scrubbing
  viewer-tools viewer-recovery gpu-recovery precomp compositions scene3d
  agent-canvas-capture opaque-capture-parity
)
ENGINE_SPECS=(native-engine native-engine-tracer media-store engine-export)
# Engine specs added after the Phase 2 gate (reported apart).
ENGINE_SPECS_NEW=(engine-present-recovery native-engine-transport engine-delivery native-edit-kernels)

PERF_RUNS="tracer-drag-today-500-idle,tracer-drag-engine-500-idle,tracer-drag-today-500-load,tracer-drag-engine-500-load,tracer-edit-playback-engine-1000,tracer-edit-playback-today-1000,tracer-playback-engine-1000,tracer-playback-engine-25,tracer-playback-engine-23976,tracer-playback-engine-30-load,tracer-playback-engine-30,tracer-playback-engine-25-load,tracer-playback-engine-23976-load,playback-engine-idle,playback-engine-agent-snapshots,playback-engine-agent-snapshots-busy"
S1_RUNS="app-playback-today-idle,app-playback-engine-idle,app-playback-today-load,app-playback-engine-load,app-latency-today-idle,app-latency-engine-idle,app-latency-today-load,app-latency-engine-load"

files() { local f; for f in "$@"; do printf 'e2e/%s.spec.ts ' "$f"; done; }
pw() { # <name> <env...> -- <playwright args...>
  local name="$1"; shift
  local envs=()
  while [ "$1" != "--" ]; do envs+=("$1"); shift; done; shift
  env "${envs[@]}" PLAYWRIGHT_JSON_OUTPUT_NAME="$OUT/$name.json" \
    "$L" bunx playwright test "$@" --workers=1 --retries=0 --global-timeout=0 --reporter=list,json
}

# Presentation and page-timing steps measure only with the main display awake
# and the session unlocked (pm-bridge's harness feature reports both); wait
# for that, then let the panel settle. GATE_NO_DISPLAY_WAIT=1 skips the wait.
wait_display() {
  [ -n "${GATE_NO_DISPLAY_WAIT:-}" ] && return 0
  local addon="$OUT/pm_bridge_harness.node"
  cp "$repo/engine/target/release/libpm_bridge_addon.dylib" "$addon" || return 1
  local state
  while :; do
    state="$(node -e "const s=require(process.argv[1]).displayState();process.stdout.write(!s.asleep&&!s.locked?'usable':'asleep '+s.asleep+' locked '+s.locked)" "$addon")"
    [ "$state" = usable ] && break
    sleep 60
  done
  sleep 20
}

step() {
  local name="$1"
  local before after start status
  case "$name" in
    tracer-measure|perf|s1|frame-bridge-integration|engine-specs-new|transport-repeat) wait_display ;;
  esac
  before="$(load1)"; start="$(date '+%Y-%m-%dT%H:%M:%S')"
  case "$name" in
    ledger-off)
      pw ledger-off POWERMOVE_NEW_ENGINE=0 -- --config playwright.ledger.config.ts ;;
    audit-suite)
      rm -f "$OUT/audit-suite.jsonl"
      pw audit-suite VITE_POWERMOVE_NATIVE_EVALUATOR=audit POWERMOVE_ENGINE_FONTS=1 \
        POWERMOVE_NATIVE_REPORT_FILE="$OUT/audit-suite.jsonl" -- &&
      node scripts/native-evaluator-report.mjs "$OUT/audit-suite.jsonl" --results "$OUT/audit-suite.json" ;;
    engine-specs)
      pw engine-specs X=1 -- $(files "${ENGINE_SPECS[@]}") ;;
    engine-specs-new)
      pw engine-specs-new X=1 -- $(files "${ENGINE_SPECS_NEW[@]}") ;;
    flag-on-specs)
      pw flag-on-specs POWERMOVE_NEW_ENGINE=1 -- $(files "${FLAG_ON_SPECS[@]}") ;;
    tracer-measure)
      rm -f "$OUT/tracer-measure-page.jsonl"
      pw tracer-measure PM_TRACER_MEASURE=1 PM_TRACER_MEASURE_OUT="$OUT/tracer-measure-page.jsonl" -- \
        e2e/native-engine-tracer-measure.spec.ts ;;
    transport-repeat)
      pw transport-repeat X=1 -- e2e/native-engine-transport.spec.ts --repeat-each="${GATE_REPEAT:-3}" ;;
    perf)
      "$L" node scripts/frame-bridge-measure.mjs --skip-build --tag "$TAG" --only "${GATE_RUNS:-$PERF_RUNS}" ;;
    s1)
      "$L" node scripts/frame-bridge-measure.mjs --skip-build --tag "$TAG" --only "${GATE_RUNS:-$S1_RUNS}" ;;
    frame-bridge-integration)
      "$L" node scripts/frame-bridge-integration.mjs --skip-build --json "$OUT/frame-bridge-integration.json" ;;
    engine-host-integration)
      "$L" node scripts/engine-host-integration.mjs --skip-build ;;  # a build would replace the harness pm-bridge
    test-mcp)
      "$L" node scripts/test-mcp.mjs --desktop ;;
    engine-switch-corpus)
      "$L" node scripts/gate-engine-switch-corpus.mjs --json "$OUT/engine-switch-corpus.json" ;;
    pm-wasm-bench)
      "$L" node scripts/pm-wasm-bench.mjs --rounds 200 ;;
    list)
      echo "ledger-off audit-suite engine-specs engine-specs-new transport-repeat flag-on-specs tracer-measure perf s1"
      echo "frame-bridge-integration engine-host-integration test-mcp engine-switch-corpus pm-wasm-bench"
      return 0 ;;
    *) echo "unknown step $name" >&2; return 2 ;;
  esac
  status=$?
  after="$(load1)"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$start" "$(date '+%Y-%m-%dT%H:%M:%S')" "$before" "$after" "$status" >> "$OUT/steps.tsv"
  return "$status"
}

for s in "$@"; do
  step "$s" > >(tee "$OUT/$s.log") 2>&1
done
