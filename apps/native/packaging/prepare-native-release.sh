#!/bin/bash
# Prepare the reviewed inputs and bundle without access to CI signing secrets.
set -euo pipefail
packaging="$(cd "$(dirname "$0")" && pwd -P)"
stage="${PM_NATIVE_STAGE:?}"
ffmpeg_out="${PM_FFMPEG_OUT:?}"

case "${1:-}" in
  inputs)
    mkdir -p "$stage/sparkle" "$stage/oidn"
    sparkle_url="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["archiveURL"])' "$packaging/sparkle-pin.json")"
    curl --fail --silent --show-error --location "$sparkle_url" -o "$stage/sparkle.tar.xz"
    python3 "$packaging/package.py" verify-sparkle-archive "$stage/sparkle.tar.xz"
    tar -xJf "$stage/sparkle.tar.xz" -C "$stage/sparkle"
    curl --fail --silent --show-error --location \
      https://github.com/RenderKit/oidn/releases/download/v2.5.1/oidn-2.5.1.arm64.macos.tar.gz \
      -o "$stage/oidn.tar.gz"
    (cd "$stage" && shasum -a 256 -c <<'HASHES'
98e0aca8e7ab69e9f4f0191582a500fd8b0d9085d68662f18ab6469e934a6efd  oidn.tar.gz
HASHES
    )
    tar -xzf "$stage/oidn.tar.gz" --strip-components 1 -C "$stage/oidn"
    ;;
  bundle)
    engine/third_party/ffmpeg/package.sh "$ffmpeg_out" "$stage/ffmpeg"
    python3 "$packaging/native-release-inputs.py" "$stage"
    public_key="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sparklePublicKey"])' \
      "$packaging/release/public-keys.json")"
    app="$stage/Powermove.app"
    export PM_FFMPEG_DIR="$stage/ffmpeg"
    python3 "$packaging/package.py" bundle \
      --executable apps/native/target/release/powermove-native --destination "$app" \
      --channel latest --version "${PM_NATIVE_VERSION:?}" --build "$PM_NATIVE_VERSION" \
      --electron-tag "${PM_ELECTRON_TAG:?}" --update-config "$stage/update-config.json" \
      --sparkle-framework "$stage/sparkle/Sparkle.framework" \
      --sparkle-feed 'https://cloud.trypowermove.com/native/latest/appcast.xml' \
      --sparkle-key "$public_key" --claude-runtime "$stage/runtime/claude" \
      --codex-vendor "$stage/runtime/codex"
    cp "$stage/ffmpeg/lib/"*.dylib "$app/Contents/Frameworks/"
    mkdir "$app/Contents/Resources/ffmpeg"
    cp -R "$stage/ffmpeg/share" "$stage/ffmpeg/NOTICE.txt" "$stage/ffmpeg/manifest.json" \
      "$app/Contents/Resources/ffmpeg/"
    cp "$packaging/../bundle/"{Credits.rtf,little-victory-deep.wav} "$app/Contents/Resources/"
    cp "$packaging/../crates/shell/icons/pierre/LICENSE" "$app/Contents/Resources/Pierre Icons License.txt"
    python3 "$packaging/../scripts/bundle-oidn.py" "$app" --oidn-dir "$stage/oidn"
    # The main executable links FFmpeg through Frameworks. Drop the build-tree fallback.
    install_name_tool -delete_rpath "$ffmpeg_out/lib" "$app/Contents/MacOS/powermove-native"
    ;;
  *)
    echo 'Expected inputs or bundle' >&2
    exit 2
    ;;
esac
