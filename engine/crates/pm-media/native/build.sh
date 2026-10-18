#!/usr/bin/env bash
# Builds the native libraries pm-media links statically: dav1d (AV1 decode
# straight into Metal-shared memory, plan section 26 amendment 6), OpenEXRCore
# (OpenEXR's C library, plan section 13 "Images"), and libjpeg-turbo (JPEG
# decoded as today's viewer decodes it: the same library, accurate integer
# IDCT, fancy upsampling).
#
#   engine/crates/pm-media/native/build.sh
#
# Sources are pinned and checksummed in sources.lock. FFmpeg's own build
# (engine/third_party/ffmpeg) links dav1d into libavcodec with hidden symbols,
# so pm-media cannot call it there and builds the same pinned dav1d itself.
#
# Output (outside the repository):
#   $PM_MEDIA_NATIVE_DIR (default ~/Library/Caches/powermove-engine/media-native/out)
#     lib/libdav1d.a, lib/libOpenEXRCore*.a (plus OpenEXR's vendored deflate,
#     zstd, and OpenJPH objects), lib/libturbojpeg.a, include/, stamp (the
#     sources.lock checksum)
#
# Environment: PM_MEDIA_NATIVE_DIR, PM_MEDIA_NATIVE_WORK, PM_MEDIA_NATIVE_DOWNLOADS,
# PM_MEDIA_NATIVE_JOBS (default 4), PM_MEDIA_NATIVE_PYTHON (python >= 3.10 for meson).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="$HERE/sources.lock"
CACHE="$HOME/Library/Caches/powermove-engine/media-native"
OUT="${PM_MEDIA_NATIVE_DIR:-$CACHE/out}"
WORK="${PM_MEDIA_NATIVE_WORK:-$CACHE/work}"
DOWNLOADS="${PM_MEDIA_NATIVE_DOWNLOADS:-$CACHE/downloads}"
JOBS="${PM_MEDIA_NATIVE_JOBS:-4}"
DEPLOYMENT_TARGET="13.0"

log() { printf '\n==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] || die "builds on macOS arm64 only"
for tool in clang clang++ cmake ninja python3 curl shasum tar xz xcrun; do
  command -v "$tool" >/dev/null 2>&1 || die "missing build tool: $tool"
done
PYTHON="${PM_MEDIA_NATIVE_PYTHON:-$(command -v python3)}"
"$PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' \
  || die "meson needs python3 >= 3.10; set PM_MEDIA_NATIVE_PYTHON"
CARGO="$(command -v cargo)"
RUSTC_BIN="$(rustup which rustc)"
NINJA="$(command -v ninja)"
CMAKE="$(command -v cmake)"

unset CFLAGS CXXFLAGS CPPFLAGS LDFLAGS LIBS CPATH C_INCLUDE_PATH CPLUS_INCLUDE_PATH \
  LIBRARY_PATH PKG_CONFIG_PATH CMAKE_PREFIX_PATH SDKROOT MAKEFLAGS
# pkg-config must not find Homebrew libraries (it found libzstd before this).
export PKG_CONFIG_LIBDIR="/nonexistent" PKG_CONFIG_PATH=""
export LC_ALL=C TZ=UTC MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" ZERO_AR_DATE=1
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$(dirname "$NINJA"):$(dirname "$CMAKE")"
BASE_FLAGS="-arch arm64 -mmacosx-version-min=$DEPLOYMENT_TARGET -ffile-prefix-map=$WORK=/pm-media-native"

NAMES=(); VERSIONS=(); SHAS=(); DIRS=(); URLS=()
while read -r name version sha dir license url rest; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ -n "$url" && -z "$rest" ]] || die "malformed line in sources.lock for $name"
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || die "bad sha256 for $name"
  NAMES+=("$name"); VERSIONS+=("$version"); SHAS+=("$sha"); DIRS+=("$dir"); URLS+=("$url")
done < "$LOCK"

idx() { local i; for i in "${!NAMES[@]}"; do [[ "${NAMES[$i]}" == "$1" ]] && { echo "$i"; return 0; }; done; die "unknown source $1"; }
archive() { local i; i="$(idx "$1")"; printf '%s/%s' "$DOWNLOADS" "${URLS[$i]##*/}"; }
src_of() { local dir="$WORK/src/${DIRS[$(idx "$1")]}"; [[ "$1" != "skcms" ]] || dir="$dir/modules/skcms"; echo "$dir"; }

fetch() {
  local i file actual; i="$(idx "$1")"; file="$(archive "$1")"
  if [[ -f "$file" && "$(shasum -a 256 "$file" | cut -d' ' -f1)" == "${SHAS[$i]}" ]]; then return 0; fi
  log "fetch $1 ${VERSIONS[$i]}"
  mkdir -p "$DOWNLOADS"; rm -f "$file.partial"
  curl -fL --retry 3 --connect-timeout 30 --proto '=https' --tlsv1.2 -o "$file.partial" "${URLS[$i]}" \
    || die "download failed: ${URLS[$i]}"
  actual="$(shasum -a 256 "$file.partial" | cut -d' ' -f1)"
  [[ "$actual" == "${SHAS[$i]}" ]] || { rm -f "$file.partial"; die "checksum mismatch for $1: got $actual"; }
  mv "$file.partial" "$file"
}

extract() {
  local i file; i="$(idx "$1")"; file="$(archive "$1")"
  [[ "$(shasum -a 256 "$file" | cut -d' ' -f1)" == "${SHAS[$i]}" ]] || die "cached archive for $1 changed"
  tar -xf "$file" -C "$WORK/src" --no-same-owner
  [[ -d "$WORK/src/${DIRS[$i]}" ]] || die "archive for $1 did not contain ${DIRS[$i]}/"
}

# Content only (no paths), so every worktree computes the same stamp; build.rs
# compares it to decide whether to run this script.
STAMP="$(cat "$LOCK" "$HERE/build.sh" "$HERE/crabby.Cargo.lock" | shasum -a 256 | cut -d' ' -f1)"
if [[ -f "$OUT/stamp" && "$(cat "$OUT/stamp")" == "$STAMP" ]]; then
  log "up to date: $OUT"
  exit 0
fi

log "workspace $WORK"
rm -rf "$WORK"
mkdir -p "$WORK/src" "$WORK/prefix"
for n in "${NAMES[@]}"; do fetch "$n"; extract "$n"; done
PREFIX="$WORK/prefix"

log "build dav1d"
CFLAGS="$BASE_FLAGS" LDFLAGS="-arch arm64 -mmacosx-version-min=$DEPLOYMENT_TARGET" \
  "$PYTHON" -I "$(src_of meson)/meson.py" setup "$WORK/build/dav1d" "$(src_of dav1d)" \
  --prefix="$PREFIX" --libdir=lib --buildtype=release --default-library=static \
  --wrap-mode=nofallback -Db_ndebug=true -Db_staticpic=true \
  -Denable_tools=false -Denable_tests=false -Denable_examples=false -Denable_docs=false -Denable_asm=true
"$PYTHON" -I "$(src_of meson)/meson.py" compile -C "$WORK/build/dav1d" -j "$JOBS"
"$PYTHON" -I "$(src_of meson)/meson.py" install -C "$WORK/build/dav1d"

log "build OpenEXRCore"
# Only the C library is built. Imath comes from the pinned archive through
# FetchContent's source override (no network); deflate, zstd, and OpenJPH are
# OpenEXR's vendored copies.
CFLAGS="-O2 $BASE_FLAGS" CXXFLAGS="-O2 $BASE_FLAGS" "$CMAKE" -S "$(src_of openexr)" -B "$WORK/build/openexr" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
  -DCMAKE_IGNORE_PREFIX_PATH="/opt/homebrew;/usr/local" -DCMAKE_FIND_FRAMEWORK=LAST \
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTING=OFF \
  -DOPENEXR_BUILD_TOOLS=OFF -DOPENEXR_BUILD_EXAMPLES=OFF -DOPENEXR_BUILD_PYTHON=OFF -DOPENEXR_INSTALL=OFF \
  -DOPENEXR_FORCE_INTERNAL_DEFLATE=ON -DOPENEXR_FORCE_INTERNAL_ZSTD=ON -DOPENEXR_FORCE_INTERNAL_OPENJPH=ON -DOPENEXR_FORCE_INTERNAL_IMATH=ON \
  -DFETCHCONTENT_FULLY_DISCONNECTED=ON -DFETCHCONTENT_SOURCE_DIR_IMATH="$(src_of imath)" \
  -DOPENEXR_ENABLE_THREADING=ON
"$CMAKE" --build "$WORK/build/openexr" --target OpenEXRCore -j "$JOBS"

log "build libjpeg-turbo"
# Static TurboJPEG API with the NEON SIMD extensions; no tools or Java.
CFLAGS="-O2 $BASE_FLAGS" "$CMAKE" -S "$(src_of libjpeg-turbo)" -B "$WORK/build/libjpeg-turbo" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
  -DCMAKE_IGNORE_PREFIX_PATH="/opt/homebrew;/usr/local" -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" -DENABLE_SHARED=OFF -DENABLE_STATIC=ON -DWITH_TURBOJPEG=ON \
  -DWITH_JAVA=OFF -DWITH_TOOLS=OFF -DWITH_TESTS=OFF -DWITH_SIMD=ON
"$CMAKE" --build "$WORK/build/libjpeg-turbo" --target turbojpeg-static -j "$JOBS"

log "build libyuv (Chromium 152)"
CFLAGS="-O2 $BASE_FLAGS" CXXFLAGS="-O2 $BASE_FLAGS" "$CMAKE" -S "$(src_of libyuv)" -B "$WORK/build/libyuv" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
  -DCMAKE_IGNORE_PREFIX_PATH="/opt/homebrew;/usr/local" -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" -DLIBYUV_BUILD_TESTS=OFF -DLIBYUV_BUILD_TOOLS=OFF
"$CMAKE" --build "$WORK/build/libyuv" --target yuv -j "$JOBS"
mkdir -p "$PREFIX/lib/pkgconfig"
cp "$WORK/build/libyuv/libyuv.a" "$PREFIX/lib/"
cp -R "$(src_of libyuv)/include/"* "$PREFIX/include/"
cat > "$PREFIX/lib/pkgconfig/yuv.pc" <<EOF
prefix=$PREFIX
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: libyuv
Description: Chromium libyuv
Version: 0.0.1917
Libs: -L\${libdir} -lyuv
Cflags: -I\${includedir}
EOF
# Use the same pinned dav1d and libyuv, never Homebrew libraries. Cargo's
# transitive dependencies are frozen alongside the checksummed sources.
log "build CrabbyAvif (Chromium 152)"
# rust-lang/rust#157750: stripped build-time proc macros fail to load on
# macOS 27. Keep their debug info; the shipped archive uses release code.
cp "$HERE/crabby.Cargo.lock" "$(src_of crabbyavif)/Cargo.lock"
PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" PKG_CONFIG_ALL_STATIC=1 \
  CARGO_PROFILE_RELEASE_DEBUG=1 CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_DEBUG=1 \
  RUSTC="$RUSTC_BIN" RUSTC_WRAPPER="" CARGO_BUILD_JOBS="$JOBS" CARGO_TARGET_DIR="$WORK/build/crabby" \
  "$CARGO" build --locked --release --manifest-path "$(src_of crabbyavif)/Cargo.toml" \
  --lib --no-default-features --features capi,dav1d,libyuv

log "build skcms (Chromium 152)"
mkdir -p "$WORK/build/skcms"
for unit in skcms.cc src/skcms_TransformBaseline.cc; do
  clang++ -O2 $BASE_FLAGS -std=c++17 -ffp-contract=off -DSKCMS_DISABLE_HSW -DSKCMS_DISABLE_SKX \
    -I"$(src_of skcms)" -c "$(src_of skcms)/$unit" -o "$WORK/build/skcms/$(basename "$unit").o"
done
ar -rcs "$WORK/build/skcms/libskcms.a" "$WORK/build/skcms/"*.o

log "stage $OUT"
rm -rf "$OUT.partial"
mkdir -p "$OUT.partial/lib" "$OUT.partial/include/openexr"
cp "$WORK/build/skcms/libskcms.a" "$OUT.partial/lib/"
mkdir -p "$OUT.partial/include/skcms/src"
cp "$(src_of skcms)/skcms.h" "$OUT.partial/include/skcms/"
cp "$(src_of skcms)/src/skcms_public.h" "$OUT.partial/include/skcms/src/"
cp "$WORK/build/crabby/release/libcrabby_avif.a" "$OUT.partial/lib/"
cp -R "$(src_of crabbyavif)/include/avif" "$OUT.partial/include/"
cp "$PREFIX/lib/libdav1d.a" "$OUT.partial/lib/"
cp -R "$PREFIX/include/dav1d" "$OUT.partial/include/"
cp "$WORK/build/libjpeg-turbo/libturbojpeg.a" "$OUT.partial/lib/"
mkdir -p "$OUT.partial/include/turbojpeg"
cp "$(src_of libjpeg-turbo)/src/turbojpeg.h" "$OUT.partial/include/turbojpeg/"
# OpenEXRCore and every static library it was built with (vendored deflate, zstd, OpenJPH).
while IFS= read -r lib; do cp "$lib" "$OUT.partial/lib/"; done < <(find "$WORK/build/openexr" -name '*.a' -not -path '*Imath*')
cp "$(src_of openexr)"/src/lib/OpenEXRCore/openexr*.h "$OUT.partial/include/openexr/"
cp "$WORK/build/openexr/cmake/OpenEXRCore/openexr_version.h" "$OUT.partial/include/openexr/"
for h in OpenEXRConfig.h IlmThreadConfig.h IexConfig.h; do cp "$WORK/build/openexr/cmake/$h" "$OUT.partial/include/openexr/"; done
mkdir -p "$OUT.partial/include/openexr/Imath"
cp "$WORK/build/openexr/_deps/imath-build/config/ImathConfig.h" "$OUT.partial/include/openexr/Imath/"
# The public C headers must compile on their own (bindgen parses them).
clang -fsyntax-only -x c -I"$OUT.partial/include/openexr" - <<<'#include "openexr.h"' || die "staged OpenEXR headers do not compile"
clang -fsyntax-only -x c -I"$OUT.partial/include" - <<<'#include <dav1d/dav1d.h>' || die "staged dav1d headers do not compile"
clang -fsyntax-only -x c -I"$OUT.partial/include" - <<<'#include <stddef.h>
#include <turbojpeg/turbojpeg.h>' || die "staged TurboJPEG header does not compile"
[[ -f "$OUT.partial/include/openexr/openexr_config.h" ]] || die "openexr_config.h missing"
# Nothing may link outside the archives except libc++ and the system.
# Apple nm rejects Rust metadata members in libcrabby_avif.a. Audit the
# C archives here; the CrabbyAvif entry points are checked by the Rust link.
DEFINED="$(nm "$OUT.partial/lib/libdav1d.a" "$OUT.partial/lib/libturbojpeg.a" "$OUT.partial/lib/libOpenEXRCore-3_5.a" 2>/dev/null | awk '$2 == "T" { print $3 }' | sort -u)"
for sym in _ZSTD_createDCtx _libdeflate_alloc_decompressor _dav1d_open _exr_start_read _tj3Init _tj3Decompress8 _tj3Decompress12; do
  grep -qx "$sym" <<<"$DEFINED" || die "$sym is not defined in the staged archives"
done
for n in dav1d openexr libjpeg-turbo crabbyavif libyuv skcms; do
  mkdir -p "$OUT.partial/share/licenses/$n"
done
cp "$(src_of dav1d)/COPYING" "$OUT.partial/share/licenses/dav1d/"
cp "$(src_of openexr)/LICENSE.md" "$OUT.partial/share/licenses/openexr/"
cp "$(src_of libjpeg-turbo)/LICENSE.md" "$(src_of libjpeg-turbo)/README.ijg" "$OUT.partial/share/licenses/libjpeg-turbo/"
cp "$(src_of libjpeg-turbo)/src/spng/LICENSE" "$OUT.partial/share/licenses/libjpeg-turbo/spng-LICENSE"
cp "$(src_of libjpeg-turbo)/src/spng/zlib/LICENSE" "$OUT.partial/share/licenses/libjpeg-turbo/zlib-LICENSE"
cp "$(src_of crabbyavif)/LICENSE" "$OUT.partial/share/licenses/crabbyavif/"
cp "$(src_of libyuv)/LICENSE" "$OUT.partial/share/licenses/libyuv/"
cp "$HERE/skcms.LICENSE" "$OUT.partial/share/licenses/skcms/LICENSE"
ls "$OUT.partial/lib" > "$OUT.partial/libs.txt"
echo "$STAMP" > "$OUT.partial/stamp"
rm -rf "$OUT"
mv "$OUT.partial" "$OUT"
rm -rf "$WORK"
log "done: $OUT"
cat "$OUT/libs.txt"
