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
# Agent notifications play this as their own sound (S114).
cp "$here/little-victory-deep.wav" "$app/Contents/Resources/"
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
# Speech to text (Captions, Transcription): the transcribe.cpp runtime folder
# (contract.json, libtranscribe and ggml). A release ships it in
# Resources/transcribe; a dev bundle copies it from PM_TRANSCRIBE_RUNTIME.
if [ -n "${PM_TRANSCRIBE_RUNTIME:-}" ] && [ -f "$PM_TRANSCRIBE_RUNTIME/contract.json" ]; then
  mkdir -p "$app/Contents/Resources/transcribe"
  cp -c "$PM_TRANSCRIBE_RUNTIME"/contract.json "$PM_TRANSCRIBE_RUNTIME"/*.dylib "$app/Contents/Resources/transcribe/" 2>/dev/null \
    || cp "$PM_TRANSCRIBE_RUNTIME"/contract.json "$PM_TRANSCRIBE_RUNTIME"/*.dylib "$app/Contents/Resources/transcribe/"
else
  echo "warning: PM_TRANSCRIBE_RUNTIME is unset; Generate Captions will say its runtime is missing." >&2
fi
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
# Review launches can hand the app environment, which LaunchServices does not
# pass from the launcher: PM_DEV_BUNDLE_ENV="PM_NATIVE_APPEARANCE=light ..."
# becomes the bundle's LSEnvironment. Unset, the bundle has none.
for pair in ${PM_DEV_BUNDLE_ENV:-}; do
  /usr/libexec/PlistBuddy -c "Add :LSEnvironment dict" "$app/Contents/Info.plist" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Add :LSEnvironment:${pair%%=*} string ${pair#*=}" "$app/Contents/Info.plist"
done
# A linked OIDN runtime is required at launch, including in export children.
# Bundle both device backends and refuse incompatible deployment targets.
if otool -L "$binary" | grep -Fq libOpenImageDenoise; then
  : "${OIDN_DIR:?Set OIDN_DIR to a Metal-enabled OIDN install with CPU fallback}"
  python3 "$(dirname "$0")/bundle-oidn.py" "$app" --oidn-dir "$OIDN_DIR"
fi
xcrun actool "$here/Assets.xcassets" --compile "$app/Contents/Resources" \
  --platform macosx --minimum-deployment-target 14.0 --accent-color AccentColor \
  --output-partial-info-plist "$out/assets-info.plist" >/dev/null
echo "$app"
