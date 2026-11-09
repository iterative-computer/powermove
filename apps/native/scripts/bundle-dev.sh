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
# The app starts these by path; without the shader compiler no effect draws.
for helper in pm-glsl-compiler pm-store-helper; do
  if [ -x "$(dirname "$binary")/$helper" ]; then
    cp "$(dirname "$binary")/$helper" "$app/Contents/MacOS/"
  else
    echo "warning: $helper is missing next to $binary; effects or Store extensions won't work." >&2
    case $helper in
      pm-glsl-compiler) echo "  build it: (cd engine && cargo +1.96.1 build --locked -p pm-glsl --bin pm-glsl-compiler) with the same CARGO_TARGET_DIR and profile" >&2 ;;
      pm-store-helper) echo "  build it: (cd apps/native && cargo +1.96.1 build --locked -p pm-runtime-v8 --features macos-sandbox --bin pm-store-helper)" >&2 ;;
    esac
  fi
done
# Agent runtimes (docs/native/AGENTS.md, launch order step 4): a release bundles
# the pinned Codex and Claude under Resources/<name>/bin/<name>. A dev bundle uses
# the ones installed on this Mac, cloned, not linked (launch refuses symlinks).
for name in codex claude; do
  if exe=$(command -v "$name" 2>/dev/null); then
    mkdir -p "$app/Contents/Resources/$name/bin"
    cp -cL "$exe" "$app/Contents/Resources/$name/bin/$name" 2>/dev/null \
      || cp -L "$exe" "$app/Contents/Resources/$name/bin/$name"
  else
    echo "warning: no $name on PATH; the Agent panel will say its runtime is missing." >&2
    echo "  install it, or set $(echo "$name" | tr a-z A-Z)_BINARY to an executable (release pins: Codex 0.159.1, Claude 2.1.280)" >&2
  fi
done
xcrun actool "$here/Assets.xcassets" --compile "$app/Contents/Resources" \
  --platform macosx --minimum-deployment-target 14.0 --accent-color AccentColor \
  --output-partial-info-plist "$out/assets-info.plist" >/dev/null
echo "$app"
