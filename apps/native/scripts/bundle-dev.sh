#!/bin/bash
# Wraps a built powermove-native binary in a dev Powermove.app so AppKit reads
# the bundle's name and AccentColor (Powermove orange) the way a release does.
#   apps/native/scripts/bundle-dev.sh <binary> <out-dir>   → <out-dir>/Powermove.app
set -euo pipefail
binary=$1
out=$2
here=$(cd "$(dirname "$0")/../bundle" && pwd)
app="$out/Powermove.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$here/Info.plist" "$app/Contents/Info.plist"
cp "$binary" "$app/Contents/MacOS/powermove-native"
xcrun actool "$here/Assets.xcassets" --compile "$app/Contents/Resources" \
  --platform macosx --minimum-deployment-target 14.0 --accent-color AccentColor \
  --output-partial-info-plist "$out/assets-info.plist" >/dev/null
echo "$app"
