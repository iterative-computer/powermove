#!/usr/bin/env bash
# Builds Powermove's FFmpeg from pinned sources for macOS arm64 or x86_64.
#
#   engine/third_party/ffmpeg/build.sh            # full build, then verify.sh
#   PM_FFMPEG_JOBS=8 engine/third_party/ffmpeg/build.sh
#
# LGPL components only: no --enable-gpl, no --enable-nonfree, no x264 or x265,
# and no FFmpeg ProRes encoders (ProRes encode goes through VideoToolbox,
# decision 12). Every dependency is built from a pinned, checksummed source as a
# static library, so the output depends only on macOS system libraries.
#
# Output (gitignored), default engine/third_party/ffmpeg/out/:
#   lib/       libav*.dylib and libsw*.dylib, install names @rpath/<name>
#   bin/       ffmpeg and ffprobe from the same configuration
#   include/   headers, lib/pkgconfig/ relocatable .pc files (for bindgen)
#   share/licenses/<component>/   license texts for every shipped component
#   manifest.json   versions, checksums, configure line, toolchain, outputs
#
# Environment overrides:
#   PM_FFMPEG_ARCH       arm64 (default) or x86_64
#   PM_FFMPEG_OUT        output directory           (default: <here>/out)
#   PM_FFMPEG_WORK       scratch build directory    (default: <here>/work)
#   PM_FFMPEG_DOWNLOADS  verified source cache      (default: <here>/downloads)
#   PM_FFMPEG_JOBS       parallel jobs              (default: CPU count - 2)
#   PM_FFMPEG_PYTHON     python >= 3.10 for meson   (default: python3 on PATH)
#   PM_FFMPEG_SKIP_VERIFY=1   skip the verify.sh run at the end
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARCH="${PM_FFMPEG_ARCH:-arm64}"
case "$ARCH" in arm64) FFMPEG_ARCH=aarch64 ;; x86_64) FFMPEG_ARCH=x86_64 ;; *) echo "Unsupported PM_FFMPEG_ARCH: $ARCH" >&2; exit 1 ;; esac
OUT="${PM_FFMPEG_OUT:-$HERE/out}"
WORK="${PM_FFMPEG_WORK:-$HERE/work}"
DOWNLOADS="${PM_FFMPEG_DOWNLOADS:-$HERE/downloads}"
LOCK="$HERE/sources.lock"

# The deployment target follows Electron 44's minimum (LSMinimumSystemVersion 13.0).
DEPLOYMENT_TARGET="13.0"
# The install prefix compiled into the binaries. It is never used at run time
# (libraries load through @rpath); a fixed value keeps local paths out of the
# shipped files and out of `ffmpeg -buildconf`.
FAKE_PREFIX="/powermove/ffmpeg"

log() { printf '\n==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# ── Preflight ────────────────────────────────────────────────────────────────

[[ "$(uname -s)" == "Darwin" ]] || die "this script builds for macOS only"
[[ "$(uname -m)" == "arm64" || "$(uname -m)" == "x86_64" ]] || die "unsupported build host"
if [[ "$ARCH" == x86_64 ]]; then command -v nasm >/dev/null || die "x86_64 requires nasm"; fi

for tool in clang clang++ make cmake ninja pkg-config python3 curl shasum tar xz autoconf automake glibtoolize xcrun otool; do
  command -v "$tool" >/dev/null 2>&1 || die "missing build tool: $tool (see README.md, Build requirements)"
done
# Resolved before PATH is reordered below (macOS's /usr/bin/python3 is too old for meson).
PYTHON="${PM_FFMPEG_PYTHON:-$(command -v python3)}"
"$PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' \
  || die "meson needs python3 >= 3.10 (found $("$PYTHON" --version 2>&1)); set PM_FFMPEG_PYTHON"

if [[ -n "${PM_FFMPEG_JOBS:-}" ]]; then
  JOBS="$PM_FFMPEG_JOBS"
else
  JOBS=$(( $(sysctl -n hw.ncpu) - 2 ))
  (( JOBS >= 1 )) || JOBS=1
fi

# A clean, deterministic environment: nothing from the caller's shell leaks
# into compiler flags, and no Homebrew library can be discovered.
unset CFLAGS CXXFLAGS CPPFLAGS LDFLAGS OBJCFLAGS LIBS CPATH C_INCLUDE_PATH CPLUS_INCLUDE_PATH \
  LIBRARY_PATH DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH PKG_CONFIG_PATH CMAKE_PREFIX_PATH \
  SDKROOT MAKEFLAGS
export LC_ALL=C TZ=UTC
# System tools first; the extra tools above are found after them.
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
export MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET"
export ZERO_AR_DATE=1
SDK_PATH="$(xcrun --sdk macosx --show-sdk-path)"
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"

PREFIX="$WORK/prefix"          # static dependencies
SRC="$WORK/src"
STAGE="$WORK/stage"
TOOLS="$WORK/tools"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
# __FILE__ and similar strings in dependencies never carry local paths.
PATH_MAP="-ffile-prefix-map=$WORK=/pm-ffmpeg"
BASE_FLAGS="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET $PATH_MAP"

# ── Sources ──────────────────────────────────────────────────────────────────

# Parallel arrays (macOS ships bash 3.2, which has no associative arrays).
NAMES=(); VERSIONS=(); SHAS=(); DIRS=(); LICENSES=(); LICENSE_FILES=(); URLS=()
while read -r name version sha dir license license_files url rest; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ -n "$url" && -z "$rest" ]] || die "malformed line in sources.lock for $name"
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || die "bad sha256 for $name in sources.lock"
  NAMES+=("$name"); VERSIONS+=("$version"); SHAS+=("$sha"); DIRS+=("$dir")
  LICENSES+=("$license"); LICENSE_FILES+=("$license_files"); URLS+=("$url")
done < "$LOCK"

idx() {
  local i
  for i in "${!NAMES[@]}"; do [[ "${NAMES[$i]}" == "$1" ]] && { echo "$i"; return 0; }; done
  die "unknown source: $1"
}
version_of() { echo "${VERSIONS[$(idx "$1")]}"; }
src_of() { echo "$SRC/${DIRS[$(idx "$1")]}"; }
archive_path() { local i; i="$(idx "$1")"; printf '%s/%s-%s' "$DOWNLOADS" "$1" "${URLS[$i]##*/}"; }

fetch() {
  local name="$1" i file actual
  i="$(idx "$name")"; file="$(archive_path "$name")"
  if [[ -f "$file" ]] && [[ "$(shasum -a 256 "$file" | cut -d' ' -f1)" == "${SHAS[$i]}" ]]; then
    return 0
  fi
  log "fetch $name ${VERSIONS[$i]}"
  mkdir -p "$DOWNLOADS"
  rm -f "$file" "$file.partial"
  curl -fL --retry 3 --connect-timeout 30 --proto '=https' --tlsv1.2 -o "$file.partial" "${URLS[$i]}" \
    || die "download failed: ${URLS[$i]}"
  actual="$(shasum -a 256 "$file.partial" | cut -d' ' -f1)"
  if [[ "$actual" != "${SHAS[$i]}" ]]; then
    rm -f "$file.partial"
    die "checksum mismatch for $name: expected ${SHAS[$i]}, got $actual"
  fi
  mv "$file.partial" "$file"
}

extract() {
  local name="$1" i file
  i="$(idx "$name")"; file="$(archive_path "$name")"
  # Re-verify: the cache may have been touched since fetch.
  [[ "$(shasum -a 256 "$file" | cut -d' ' -f1)" == "${SHAS[$i]}" ]] || die "cached archive for $name changed; delete $file"
  rm -rf "${SRC:?}/${DIRS[$i]}"
  tar -xf "$file" -C "$SRC" --no-same-owner
  [[ -d "$SRC/${DIRS[$i]}" ]] || die "archive for $name did not contain ${DIRS[$i]}/"
}

log "workspace $WORK"
rm -rf "$WORK"
mkdir -p "$PREFIX/lib/pkgconfig" "$SRC" "$STAGE" "$TOOLS"
for name in "${NAMES[@]}"; do fetch "$name"; extract "$name"; done

MESON=("$PYTHON" -I "$(src_of meson)/meson.py")
MESON_COMMON=(--prefix="$PREFIX" --libdir=lib --buildtype=release --default-library=static
  --wrap-mode=nofallback --auto-features=disabled -Db_ndebug=true -Db_staticpic=true)
CMAKE_COMMON=(-G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" -DCMAKE_INSTALL_LIBDIR=lib
  -DCMAKE_OSX_ARCHITECTURES="$ARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET"
  -DCMAKE_PREFIX_PATH="$PREFIX" -DCMAKE_IGNORE_PREFIX_PATH="/opt/homebrew;/usr/local" -DCMAKE_FIND_FRAMEWORK=LAST
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF)

if [[ "$ARCH" != "$(uname -m)" ]]; then
  cat > "$WORK/cross.ini" <<EOF
[binaries]
c = ['clang', '-arch', '$ARCH']
cpp = ['clang++', '-arch', '$ARCH']
ar = 'ar'
strip = 'strip'
pkg-config = 'pkg-config'
[host_machine]
system = 'darwin'
cpu_family = '$ARCH'
cpu = '$ARCH'
endian = 'little'
[properties]
needs_exe_wrapper = false
EOF
  MESON_COMMON+=(--cross-file="$WORK/cross.ini")
fi

with_flags() {
  CFLAGS="-O2 $BASE_FLAGS" CXXFLAGS="-O2 $BASE_FLAGS" LDFLAGS="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET" "$@"
}

# ── Static dependencies ──────────────────────────────────────────────────────

build_zlib() {
  log "build zlib"
  cd "$(src_of zlib)"
  with_flags ./configure --static --prefix="$PREFIX" --libdir="$PREFIX/lib"
  make -j"$JOBS" libz.a
  make install
}

build_libvpx() {
  log "build libvpx"
  mkdir -p "$WORK/build/libvpx" && cd "$WORK/build/libvpx"
  # libvpx compiles its configure arguments into the library
  # (vpx_codec_build_config), so no path is passed to configure: the prefix is
  # given to make instead. darwin22 = macOS 13; that target only adds -arch.
  CFLAGS="$BASE_FLAGS" CXXFLAGS="$BASE_FLAGS" LDFLAGS="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET" \
    "$(src_of libvpx)/configure" --target="$ARCH-darwin22-gcc" \
    --enable-static --disable-shared --enable-pic --enable-vp8 --enable-vp9 --enable-vp9-highbitdepth \
    --enable-runtime-cpu-detect --disable-examples --disable-tools --disable-docs --disable-unit-tests \
    --disable-debug --disable-install-bins --disable-install-srcs
  make -j"$JOBS" PREFIX="$PREFIX"
  make install PREFIX="$PREFIX" DIST_DIR="$PREFIX"
}

build_opus() {
  log "build opus"
  cd "$(src_of opus)"
  with_flags ./configure --host="$FFMPEG_ARCH-apple-darwin" --prefix="$PREFIX" --libdir="$PREFIX/lib" --enable-static --disable-shared --with-pic \
    --disable-doc --disable-extra-programs
  make -j"$JOBS"
  make install
}

build_dav1d() {
  log "build dav1d"
  CFLAGS="$BASE_FLAGS" LDFLAGS="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET" \
    "${MESON[@]}" setup "$WORK/build/dav1d" "$(src_of dav1d)" "${MESON_COMMON[@]}" \
    -Denable_tools=false -Denable_tests=false -Denable_examples=false -Denable_docs=false -Denable_asm=true
  "${MESON[@]}" install -C "$WORK/build/dav1d"
}

build_svtav1() {
  log "build SVT-AV1"
  CFLAGS="$BASE_FLAGS" CXXFLAGS="$BASE_FLAGS" cmake -S "$(src_of svt-av1)" -B "$WORK/build/svt-av1" "${CMAKE_COMMON[@]}" \
    -DBUILD_APPS=OFF -DBUILD_TESTING=OFF -DSVT_AV1_LTO=OFF -DREPRODUCIBLE_BUILDS=ON -DEXCLUDE_HASH=ON
  cmake --build "$WORK/build/svt-av1" -j "$JOBS"
  cmake --install "$WORK/build/svt-av1"
}

build_freetype() {
  log "build freetype"
  # zlib from our prefix; no png, bzip2, brotli, or harfbuzz (drawtext needs none).
  CFLAGS="$BASE_FLAGS" LDFLAGS="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET" \
    "${MESON[@]}" setup "$WORK/build/freetype" "$(src_of freetype)" "${MESON_COMMON[@]}" \
    -Dzlib=system -Dpng=disabled -Dbzip2=disabled -Dbrotli=disabled -Dharfbuzz=disabled -Dtests=disabled -Dmmap=enabled
  "${MESON[@]}" install -C "$WORK/build/freetype"
}

build_harfbuzz() {
  log "build harfbuzz"
  # FFmpeg's drawtext shapes text with harfbuzz over freetype (hb-ft).
  CFLAGS="$BASE_FLAGS" CXXFLAGS="$BASE_FLAGS" LDFLAGS="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET" \
    "${MESON[@]}" setup "$WORK/build/harfbuzz" "$(src_of harfbuzz)" "${MESON_COMMON[@]}" \
    -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled -Dchafa=disabled -Dicu=disabled \
    -Dgraphite=disabled -Dgraphite2=disabled -Dcoretext=disabled -Draster=disabled -Dvector=disabled \
    -Dsubset=disabled -Dtests=disabled -Dintrospection=disabled -Ddocs=disabled -Dutilities=disabled \
    -Dbenchmark=disabled
  "${MESON[@]}" install -C "$WORK/build/harfbuzz"
}

build_zimg() {
  log "build zimg"
  cd "$(src_of zimg)"
  # zimg ships no release tarball with a generated configure, so autogen.sh runs the
  # host's autoconf, automake and libtool (versions recorded in the manifest).
  # MKDIR_P keeps configure from picking up GNU mkdir (gmkdir) when coreutils is installed.
  LIBTOOLIZE=glibtoolize ./autogen.sh >/dev/null
  MKDIR_P="mkdir -p" with_flags ./configure --host="$FFMPEG_ARCH-apple-darwin" --prefix="$PREFIX" --libdir="$PREFIX/lib" --enable-static --disable-shared --with-pic \
    --disable-testapp --disable-example --disable-unit-test
  make -j"$JOBS"
  make install
  # zimg.pc names libstdc++, which macOS no longer ships; the C++ runtime is libc++.
  sed -i '' -e 's/-lstdc++/-lc++/g' "$PREFIX/lib/pkgconfig/zimg.pc"
}

# Each step in a subshell, so a failed `cd` cannot leak into the next one.
for step in zlib libvpx opus dav1d svtav1 freetype harfbuzz zimg; do
  ( "build_$step" )
done

# Static libraries only: a stray dylib in the prefix would be linked instead.
if compgen -G "$PREFIX/lib/*.dylib" >/dev/null; then
  die "a dependency installed a dylib: $(ls "$PREFIX"/lib/*.dylib)"
fi

# ── FFmpeg ───────────────────────────────────────────────────────────────────

CONFIGURE_FLAGS=(
  --prefix="$FAKE_PREFIX"
  --install-name-dir=@rpath
  --arch="$FFMPEG_ARCH" --target-os=darwin --cc=clang --cxx=clang++
  --enable-shared --disable-static
  --disable-autodetect
  --enable-pthreads
  --enable-videotoolbox --enable-audiotoolbox
  --enable-zlib --enable-bzlib --enable-iconv
  --enable-libvpx --enable-libopus --enable-libdav1d --enable-libsvtav1
  --enable-libfreetype --enable-libharfbuzz --enable-libzimg
  --disable-encoder=prores,prores_aw,prores_ks
  --disable-decoder=prores_raw
  --disable-ffplay --disable-doc --disable-debug
  --pkg-config=pkg-config --pkg-config-flags=--static
  --extra-cflags="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET"
  --extra-ldflags="-arch $ARCH -mmacosx-version-min=$DEPLOYMENT_TARGET"
  --extra-ldexeflags="-Wl,-rpath,@executable_path/../lib"
  --extra-ldsoflags="-Wl,-rpath,@loader_path"
  # libc++ for harfbuzz and zimg (C++); libiconv because the iconv probe passes
  # without it but libavcodec's subtitle decoding needs it at link time.
  --extra-libs="-lc++ -liconv"
)

if [[ "$ARCH" != "$(uname -m)" ]]; then CONFIGURE_FLAGS+=(--enable-cross-compile); fi
export PM_FFMPEG_ARCH="$ARCH"

log "configure FFmpeg $(version_of ffmpeg)"
cd "$(src_of ffmpeg)"
# In-tree build: sources are compiled by relative path, so no local path reaches
# __FILE__ strings, and the configure line above carries none either.
if ! ./configure "${CONFIGURE_FLAGS[@]}" > "$WORK/ffmpeg-configure.log" 2>&1; then
  tail -n 40 ffbuild/config.log >&2 || true
  die "FFmpeg configure failed (log: $WORK/ffmpeg-configure.log, ffbuild/config.log)"
fi
for forbidden in CONFIG_GPL CONFIG_NONFREE CONFIG_LIBX264 CONFIG_LIBX265; do
  grep -q "^#define $forbidden 0$" config.h || die "$forbidden is not 0 in config.h"
done

log "build FFmpeg (jobs: $JOBS)"
make -j"$JOBS" >"$WORK/ffmpeg-make.log" 2>&1 || { tail -n 60 "$WORK/ffmpeg-make.log" >&2; die "FFmpeg build failed"; }
make install DESTDIR="$STAGE" >"$WORK/ffmpeg-install.log" 2>&1 || { tail -n 40 "$WORK/ffmpeg-install.log" >&2; die "FFmpeg install failed"; }

# ── Assemble the output ──────────────────────────────────────────────────────

log "assemble $OUT"
rm -rf "$OUT"
mkdir -p "$OUT/share/licenses"
cp -R "$STAGE$FAKE_PREFIX/bin" "$STAGE$FAKE_PREFIX/lib" "$STAGE$FAKE_PREFIX/include" "$OUT/"
# Relocatable pkg-config files: the prefix follows wherever out/ is copied. The
# static dependencies are inside the dylibs, so Libs.private (which names them,
# with -L into the work directory) is emptied; it only matters to static links.
for pc in "$OUT"/lib/pkgconfig/*.pc; do
  sed -i '' \
    -e 's|^prefix=.*$|prefix=${pcfiledir}/../..|' \
    -e 's|^libdir=.*$|libdir=${prefix}/lib|' \
    -e 's|^includedir=.*$|includedir=${prefix}/include|' \
    -e 's|^Libs.private:.*$|Libs.private:|' \
    "$pc"
done
for i in "${!NAMES[@]}"; do
  name="${NAMES[$i]}"
  [[ "$name" == meson ]] && continue
  mkdir -p "$OUT/share/licenses/$name"
  IFS=',' read -r -a files <<<"${LICENSE_FILES[$i]}"
  for file in "${files[@]}"; do
    cp "$SRC/${DIRS[$i]}/$file" "$OUT/share/licenses/$name/$(basename "$file")"
  done
done

# install_name_tool and the linker may leave Intel Mach-O binaries unsigned.
# Sign after all binary edits and before recording their checksums.
for binary in "$OUT"/bin/ffmpeg "$OUT"/bin/ffprobe "$OUT"/lib/*.dylib; do
  [[ -L "$binary" ]] && continue
  codesign --force --sign - "$binary"
done

# ── Manifest ─────────────────────────────────────────────────────────────────

sources_tsv="$WORK/sources.tsv"
: > "$sources_tsv"
for i in "${!NAMES[@]}"; do
  role=shipped; [[ "${NAMES[$i]}" == meson ]] && role=build-tool
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "${NAMES[$i]}" "${VERSIONS[$i]}" "${SHAS[$i]}" "${URLS[$i]}" "${LICENSES[$i]}" "$role" >> "$sources_tsv"
done
{
  printf 'clang\t%s\n' "$(clang --version | head -n 1)"
  printf 'sdk\tmacosx %s\n' "$SDK_VERSION"
  printf 'deployment_target\t%s\n' "$DEPLOYMENT_TARGET"
  printf 'cmake\t%s\n' "$(cmake --version | head -n 1)"
  printf 'ninja\t%s\n' "$(ninja --version)"
  printf 'meson\t%s\n' "$("${MESON[@]}" --version)"
  printf 'python3\t%s\n' "$("$PYTHON" --version 2>&1)"
  printf 'pkg-config\t%s\n' "$(pkg-config --version)"
  printf 'autoconf\t%s\n' "$(autoconf --version | head -n 1)"
  printf 'automake\t%s\n' "$(automake --version | head -n 1)"
  printf 'libtool\t%s\n' "$(glibtoolize --version | head -n 1)"
  printf 'host\t%s\n' "$(uname -srm)"
} > "$WORK/toolchain.tsv"

DYLD_LIBRARY_PATH= "$OUT/bin/ffmpeg" -hide_banner -buildconf > "$WORK/buildconf.txt"
"$PYTHON" -I "$HERE/manifest.py" \
  --out "$OUT" --sources "$sources_tsv" --toolchain "$WORK/toolchain.tsv" \
  --configure-flags "$(printf '%s\n' "${CONFIGURE_FLAGS[@]}")" \
  --buildconf "$WORK/buildconf.txt" --ffmpeg-version "$(version_of ffmpeg)"

log "done: $OUT"

if [[ "${PM_FFMPEG_SKIP_VERIFY:-0}" != 1 ]]; then
  "$HERE/verify.sh" "$OUT"
fi
