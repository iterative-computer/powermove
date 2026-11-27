#!/bin/zsh
# Ready for a separately scheduled foreground quiet window. Does not unlock.
set -euo pipefail
cd "${0:A:h}/../../../.."
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
echo "S3 build begins $(date -u +%FT%TZ)" >&2
export S3_PROFILE=${S3_PROFILE:-debug}
build_flags=()
if [[ "$S3_PROFILE" == release ]]; then build_flags+=(--release); fi
python3 apps/native/spikes/s3/build.py build -p pm-native-s3-spike --bin pm-native-s3-spike "${build_flags[@]}"
echo "S3 build done; starting separately locked foreground cases" >&2
for hz in 120 60; do
  zsh apps/native/spikes/s3/run.sh --seconds=60 --hz=$hz --report=/tmp/s3b-foreground-$hz.json
done
python3 apps/native/spikes/s3/input.py
python3 apps/native/spikes/s3/input.py --cached
zsh apps/native/spikes/s3/run.sh --seconds=9 --synthetic-scrub --report=/tmp/s3b-foreground-cached.json
zsh apps/native/spikes/s3/run.sh --verify --seconds=3 \
  --pq="$PWD/engine/crates/pm-render/tests/fixtures/video/hevc10-2020-pq.mp4" \
  --report=/tmp/s3b-foreground-accuracy.json
