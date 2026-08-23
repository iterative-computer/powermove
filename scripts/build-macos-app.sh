#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD=${PM_BUILD_DIR:-"$ROOT/build"}
APP="$BUILD/Powermove.app"
CONTENTS="$APP/Contents"
MACOS="$CONTENTS/MacOS"
RES="$CONTENTS/Resources"
WEB="$RES/web"
INSTALL=${1:-}
BUNDLE_ID=${PM_BUNDLE_ID:-com.zellzoi.powermove.dev}
if [ "$INSTALL" = "--release" ] || [ "$INSTALL" = "--install" ]; then
  BUNDLE_ID=com.zellzoi.powermove
fi
export CLANG_MODULE_CACHE_PATH="$BUILD/.module-cache"
export SWIFT_MODULECACHE_PATH="$BUILD/.module-cache"

rm -rf "$APP"
mkdir -p "$MACOS" "$WEB/css" "$WEB/js" "$WEB/assets/fonts" "$BUILD/Powermove.iconset" "$BUILD/.module-cache"

xcrun swiftc -O -framework Cocoa -framework WebKit "$ROOT/native/main.swift" -o "$MACOS/Powermove"
if [ -f "$ROOT/assets/Powermove.icns" ]; then
  cp "$ROOT/assets/Powermove.icns" "$RES/Powermove.icns"
  xattr -c "$RES/Powermove.icns" 2>/dev/null || true
else
  xcrun swift "$ROOT/native/make-icon.swift" "$BUILD/icon-1024.png"
  for spec in "16 icon_16x16.png" "32 icon_16x16@2x.png" "32 icon_32x32.png" "64 icon_32x32@2x.png" "128 icon_128x128.png" "256 icon_128x128@2x.png" "256 icon_256x256.png" "512 icon_256x256@2x.png" "512 icon_512x512.png" "1024 icon_512x512@2x.png"; do
    set -- $spec
    sips -z "$1" "$1" "$BUILD/icon-1024.png" --out "$BUILD/Powermove.iconset/$2" >/dev/null
  done
  iconutil -c icns "$BUILD/Powermove.iconset" -o "$RES/Powermove.icns"
fi
cp "$ROOT/index.html" "$WEB/index.html"
cp -R "$ROOT/css/." "$WEB/css/"
cp -R "$ROOT/js/." "$WEB/js/"
cp -R "$ROOT/assets/fonts/." "$WEB/assets/fonts/"

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleDisplayName</key><string>Powermove</string>
  <key>CFBundleExecutable</key><string>Powermove</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleIconFile</key><string>Powermove</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Powermove</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSHumanReadableCopyright</key><string>Copyright © 2026 Motioner. All rights reserved.</string>
  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeName</key><string>Powermove Project</string>
    <key>CFBundleTypeExtensions</key><array><string>pmv</string></array>
    <key>CFBundleTypeRole</key><string>Editor</string>
  </dict></array>
</dict></plist>
PLIST

xattr -cr "$APP" 2>/dev/null || true
# File Provider can retain bundle-level metadata even after a recursive clear.
# Remove those attributes explicitly so strict code-sign verification is reliable.
xattr -d com.apple.FinderInfo "$APP" 2>/dev/null || true
xattr -d 'com.apple.fileprovider.fpfs#P' "$APP" 2>/dev/null || true
codesign --force --deep --sign - "$APP" >/dev/null
if [ "$INSTALL" = "--install" ]; then
  rm -rf /Applications/Powermove.app
  ditto "$APP" /Applications/Powermove.app
  xattr -dr com.apple.quarantine /Applications/Powermove.app 2>/dev/null || true
  echo "/Applications/Powermove.app"
else
  echo "$APP"
fi
