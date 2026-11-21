#!/bin/sh
# Full Electron vs Native campaign on one Mac. Paths are parameters so it runs on any machine.
# Usage: run_all.sh <Electron Powermove.app> <powermove-native binary> <electron apps/desktop dir> <media dir> <out dir> [runs=6] [first run=0]
# Renders are probed and contact-sheeted after every scene and then deleted (ProRes 4444 fills a small disk fast);
# sheets are kept in <out>/sheets. A first run > 0 resumes an interrupted campaign (scenes are not rebuilt).
#   Electron app: production build, e.g. apps/desktop/dist/mac-arm64/Powermove.app (npx electron-builder --mac --dir)
#   Native binary: apps/native/target/release/powermove-native with pm-glsl-compiler beside it
#   electron apps/desktop dir: provides playwright and @electron/fuses (node_modules)
# Run 0 is the warm-up and is excluded from the summary. Output dir should be under /tmp
# (the Native perf gate only accepts /tmp projects; scenes are copied there per run anyway).
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
EAPP=$1; NBIN=$2; EDIR=$3; MEDIA=$4; OUT=$5; RUNS=${6:-6}; FIRST=${7:-0}
export PM_ELECTRON_PKG="$EDIR/package.json"
mkdir -p "$OUT/raw" "$OUT/renders" "$OUT/scenes" "$OUT/sheets"
[ -f "$MEDIA/clip6.mp4" ] || sh "$HERE/make_media.sh" "$MEDIA"
EBIN="$OUT/Powermove-bench.app/Contents/MacOS/Powermove"
if [ "$FIRST" -eq 0 ]; then
  node "$HERE/electron_inspectable_copy.mjs" "$EAPP" "$OUT/Powermove-bench.app" "$EDIR/package.json"
  node "$HERE/electron_bench.mjs" build "$EBIN" "$MEDIA" "$OUT/scenes" > "$OUT/raw/build.log" 2>&1
fi
r=$FIRST
while [ $r -lt "$RUNS" ]; do
  for s in A B C; do
    echo "run $r scene $s $(date +%T)"
    timeout 900 node "$HERE/electron_bench.mjs" run "$EBIN" "$OUT/scenes/scene-$s.pmv" "$OUT/renders" $r --export >> "$OUT/raw/electron.log" 2>&1 \
      || echo "BENCH {\"op\":\"failed\",\"app\":\"electron\",\"run\":$r,\"scene\":\"scene-$s.pmv\",\"exit\":$?}" >> "$OUT/raw/electron.log"
    pkill -f "$OUT/Powermove-bench.app" 2>/dev/null || true
    timeout 900 python3 "$HERE/native_bench.py" --bin "$NBIN" --scene "$OUT/scenes/scene-$s.pmv" --out "$OUT/renders" --run $r --export >> "$OUT/raw/native.log" 2>&1 \
      || echo "BENCH {\"op\":\"failed\",\"app\":\"native\",\"run\":$r,\"scene\":\"scene-$s.pmv\",\"exit\":$?}" >> "$OUT/raw/native.log"
    pkill -f "$NBIN" 2>/dev/null || true
    sh "$HERE/verify_outputs.sh" "$OUT/renders" >> "$OUT/raw/verify.log" 2>&1 || true
    mv "$OUT"/renders/*.sheet.png "$OUT/sheets/" 2>/dev/null || true
    rm -f "$OUT"/renders/*.mp4 "$OUT"/renders/*.mov
  done
  r=$((r+1))
done
python3 "$HERE/parse_results.py" "$OUT/raw" "$HERE/results"
echo CAMPAIGN_DONE
