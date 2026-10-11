#!/usr/bin/env bash
# Verifies a Powermove FFmpeg build (build.sh runs this at the end).
#
#   engine/third_party/ffmpeg/verify.sh [out-dir]
#
# Checks, each reported PASS or FAIL:
#   - ffmpeg and ffprobe report the LGPL 2.1 or later (-L); their -buildconf has no
#     gpl, nonfree, or version3
#   - every binary is arm64, minimum macOS 13.0, validly signed (ad-hoc), and
#     links only @rpath and system libraries (otool -L), never the system libz
#   - every dylib's install name is @rpath/<name in lib/ that resolves to it>
#   - no local build paths (/Users/, /opt/homebrew, /usr/local, the work dir) in any binary
#   - every component in required-components.tsv is present or absent as expected
#   - no file in the output names the build machine; the .pc files are relocatable
#     and a C program built from them links and runs against the dylibs
#   - manifest.json checksums match the files
# Exits non-zero when any check fails.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$(cd "${1:-$HERE/out}" && pwd)" || { echo "error: no build output at ${1:-$HERE/out}" >&2; exit 1; }
FFMPEG="$OUT/bin/ffmpeg"
FFPROBE="$OUT/bin/ffprobe"
COMPONENTS="$HERE/required-components.tsv"
WORK="${PM_FFMPEG_WORK:-$HERE/work}"

failures=0
pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; failures=$((failures + 1)); }

[[ -x "$FFMPEG" && -x "$FFPROBE" ]] || { fail "bin/ffmpeg and bin/ffprobe exist"; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── License ──────────────────────────────────────────────────────────────────

for tool in "$FFMPEG" "$FFPROBE"; do
  name="$(basename "$tool")"
  license="$("$tool" -hide_banner -L 2>&1)"
  first="$(printf '%s\n' "$license" | awk 'BEGIN{RS=""} NR==1' | tr -s ' \n' ' ')"
  if [[ "$first" == *"GNU Lesser General Public License"* && "$first" == *"version 2.1 of the License"* ]]; then
    pass "$name -L: LGPL version 2.1 or later"
  else
    fail "$name -L does not state the LGPL 2.1: $first"
  fi
  buildconf="$("$tool" -hide_banner -buildconf 2>&1)"
  if grep -qiE 'gpl|nonfree' <<<"$buildconf"; then
    fail "$name -buildconf mentions gpl or nonfree: $(grep -ioE '[^ ]*(gpl|nonfree)[^ ]*' <<<"$buildconf" | sort -u | tr '\n' ' ')"
  else
    pass "$name -buildconf has no gpl or nonfree"
  fi
  if grep -qE 'version3' <<<"$buildconf"; then
    fail "$name -buildconf enables version3 (LGPLv3); the build is meant to stay LGPL 2.1+"
  fi
done

# ── Binaries ─────────────────────────────────────────────────────────────────

binaries=("$FFMPEG" "$FFPROBE")
while IFS= read -r lib; do binaries+=("$lib"); done < <(find "$OUT/lib" -maxdepth 1 -type f -name '*.dylib' | sort)
(( ${#binaries[@]} >= 9 )) || fail "expected 7 libraries plus 2 executables, found ${#binaries[@]} binaries"

before_binaries=$failures
for bin in "${binaries[@]}"; do
  rel="${bin#"$OUT"/}"
  archs="$(lipo -archs "$bin" 2>/dev/null)"
  [[ "$archs" == "arm64" ]] || fail "$rel architectures: '$archs' (want arm64)"
  minos="$(otool -l "$bin" | awk '/cmd LC_BUILD_VERSION/{f=1} f&&/minos/{print $2; exit}')"
  [[ "$minos" == "13.0" ]] || fail "$rel minimum macOS $minos (want 13.0)"
  codesign --verify --strict "$bin" 2>/dev/null || fail "$rel code signature is invalid"

  own=""
  if [[ "$bin" == *.dylib ]]; then
    own="$(otool -D "$bin" | sed -n 2p)"
    # FFmpeg's install name is the major-version name (libavcodec.63.dylib), a
    # symlink next to the real file; it must resolve to this very file.
    target="$OUT/lib/${own#@rpath/}"
    if [[ "$own" != @rpath/* || ! -e "$target" || "$(cd "$(dirname "$target")" && realpath "$(basename "$target")")" != "$(realpath "$bin")" ]]; then
      fail "$rel install name '$own' is not @rpath/<a name in lib/ that resolves to this file>"
    fi
  fi
  bad=""
  while IFS= read -r dep; do
    [[ -z "$dep" || "$dep" == "$own" ]] && continue
    case "$dep" in
      @rpath/*|/usr/lib/*|/System/Library/*) ;;
      *) bad+="$dep " ;;
    esac
  done < <(otool -L "$bin" | tail -n +2 | sed -E 's/^[[:space:]]+//; s/ \(compatibility.*$//')
  [[ -z "$bad" ]] || fail "$rel links non-system libraries: $bad"
  # zlib is built from the pinned source and linked statically; the system libz
  # passing the check above would mean FFmpeg's -lz probe found the SDK instead.
  if otool -L "$bin" | grep -qE '/usr/lib/libz\.'; then
    fail "$rel links the system libz instead of the pinned static zlib"
  fi

  leaks="$(strings -a "$bin" | grep -E "/Users/|/opt/homebrew|/usr/local|$WORK" | sort -u | head -n 5)"
  [[ -z "$leaks" ]] || fail "$rel contains local paths: $(tr '\n' ' ' <<<"$leaks")"
done
(( failures == before_binaries )) && pass "${#binaries[@]} binaries: arm64, macOS 13.0, signed, only @rpath and system libraries, no local paths"

# Libraries the executables and dylibs link, for the report.
printf '      linked system libraries: %s\n' "$(for bin in "${binaries[@]}"; do otool -L "$bin" | tail -n +2; done \
  | sed -E 's/^[[:space:]]+//; s/ \(compatibility.*$//' | grep -v '^@rpath/' | sort -u | tr '\n' ' ')"

# The executables run from out/bin with no environment help.
if env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH "$FFMPEG" -hide_banner -version >/dev/null 2>&1 \
   && env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH "$FFPROBE" -hide_banner -version >/dev/null 2>&1; then
  pass "ffmpeg and ffprobe load their dylibs through @executable_path/../lib"
else
  fail "ffmpeg or ffprobe cannot load its libraries from $OUT/lib"
fi

# ── Headers, pkg-config, and the rest of the tree ────────────────────────────

# No file anywhere in the output names the build machine.
tree_leaks="$(grep -rlE "/Users/|/opt/homebrew|/usr/local|$WORK" "$OUT" 2>/dev/null | sed "s|^$OUT/||" | head -n 5)"
if [[ -z "$tree_leaks" ]]; then
  pass "no file in the output contains a local path"
else
  fail "files with local paths: $(tr '\n' ' ' <<<"$tree_leaks")"
fi

# The relocatable .pc files resolve into this output wherever it is, and a
# program built from them links and runs against these dylibs.
pc() { PKG_CONFIG_LIBDIR="$OUT/lib/pkgconfig" PKG_CONFIG_PATH= pkg-config "$@"; }
pc_ok=1
for module in libavformat libavcodec libavfilter libavdevice libavutil libswscale libswresample; do
  inc="$(pc --variable=includedir "$module" 2>/dev/null)"; lib="$(pc --variable=libdir "$module" 2>/dev/null)"
  # pkgconf shell-escapes spaces in ${pcfiledir}; undo that for the comparison.
  inc="${inc//\\ / }"; lib="${lib//\\ / }"
  if [[ -z "$inc" || -z "$lib" || "$(cd "$inc" 2>/dev/null && pwd -P)" != "$(cd "$OUT/include" && pwd -P)" \
        || "$(cd "$lib" 2>/dev/null && pwd -P)" != "$(cd "$OUT/lib" && pwd -P)" ]]; then
    pc_ok=0; fail "$module.pc does not resolve into $OUT (includedir '$inc', libdir '$lib')"
  fi
done
if (( pc_ok )); then
  cat > "$TMP/link.c" <<'EOF'
#include <stdio.h>
#include <string.h>
#include <libavcodec/avcodec.h>
#include <libavfilter/avfilter.h>
#include <libavformat/avformat.h>
int main(void) {
  if (!strstr(avcodec_license(), "LGPL")) return 1;
  if (!avcodec_find_decoder_by_name("libdav1d") || !avcodec_find_encoder_by_name("libvpx-vp9")) return 2;
  if (avcodec_find_encoder_by_name("libx264") || avcodec_find_encoder_by_name("prores_ks")) return 3;
  if (!avfilter_get_by_name("drawtext") || !av_find_input_format("mov")) return 4;
  return 0;
}
EOF
  if clang -arch arm64 -mmacosx-version-min=13.0 "$TMP/link.c" -I"$inc" -L"$lib" -lavfilter -lavformat -lavcodec -lavutil \
       -Wl,-rpath,"$OUT/lib" -o "$TMP/link" 2>"$TMP/link.log" \
     && env -u DYLD_LIBRARY_PATH "$TMP/link"; then
    pass "pkg-config files are relocatable; a C program links and runs against the dylibs (LGPL, dav1d, vpx, drawtext, mov; no x264 or prores_ks)"
  else
    fail "linking a C program through pkg-config failed (exit $?): $(tail -n 3 "$TMP/link.log" 2>/dev/null | tr '\n' ' ')"
  fi
fi

# ── Components ───────────────────────────────────────────────────────────────

list_names() {
  case "$1" in
    encoder|decoder)
      "$FFMPEG" -hide_banner "-${1}s" 2>/dev/null | awk 'f && NF >= 2 {print $2} /^ *-+ *$/ {f=1}' ;;
    filter)
      "$FFMPEG" -hide_banner -filters 2>/dev/null | awk '$3 ~ /->/ {print $2}' ;;
    demuxer|muxer|device)
      "$FFMPEG" -hide_banner "-${1}s" 2>/dev/null | awk 'f && NF >= 2 {print ($1 ~ /^[DE.]+$/ ? $2 : $1)} /^ *-+ *$/ {f=1}' | tr ',' '\n' ;;
    protocol)
      "$FFMPEG" -hide_banner -protocols 2>/dev/null | awk '/^ +[a-z0-9_]+$/ {print $1}' ;;
    hwaccel)
      "$FFMPEG" -hide_banner -hwaccels 2>/dev/null | tail -n +2 | awk 'NF {print $1}' ;;
  esac
}

declare_lists() {
  for kind in protocol device demuxer muxer decoder encoder filter hwaccel; do
    list_names "$kind" | sort -u > "$TMP/$kind"
    [[ -s "$TMP/$kind" ]] || fail "could not list ${kind}s from ffmpeg"
  done
}

declare_lists

present_ok=0; present_missing=""; absent_ok=0; absent_found=""
broken=""
while IFS=$'\t' read -r kind name expect sites; do
  [[ -z "$kind" || "$kind" == \#* ]] && continue
  [[ -f "$TMP/$kind" ]] || { fail "unknown component kind '$kind' for $name"; continue; }
  if grep -qxF -- "$name" "$TMP/$kind"; then exists=1; else exists=0; fi
  case "$expect" in
    present)
      if (( exists )); then present_ok=$((present_ok + 1)); else present_missing+="$kind:$name "; fi ;;
    absent)
      if (( exists )); then absent_found+="$kind:$name "; else
        absent_ok=$((absent_ok + 1))
        [[ "$sites" == none* ]] || broken+="  $kind $name: $sites"$'\n'
      fi ;;
    *) fail "bad expectation '$expect' for $kind $name" ;;
  esac
done < "$COMPONENTS"

if [[ -z "$present_missing" ]]; then pass "$present_ok required components present"; else fail "required components missing: $present_missing"; fi
if [[ -z "$absent_found" ]]; then pass "$absent_ok excluded components absent"; else fail "excluded components present: $absent_found"; fi
if [[ -n "$broken" ]]; then
  printf '      call sites that break with this build (they use excluded components):\n%s' "$broken"
fi

# ── Manifest ─────────────────────────────────────────────────────────────────

if [[ -f "$OUT/manifest.json" ]]; then
  mismatch="$(cd "$OUT" && /usr/bin/python3 -I -c '
import hashlib, json, sys
manifest = json.load(open("manifest.json"))
for entry in manifest["outputs"]:
    digest = hashlib.sha256(open(entry["path"], "rb").read()).hexdigest()
    if digest != entry["sha256"]:
        print(entry["path"])
')"
  if [[ -z "$mismatch" ]]; then pass "manifest.json checksums match"; else fail "manifest.json checksum mismatch: $mismatch"; fi
else
  fail "manifest.json is missing"
fi

echo
if (( failures )); then
  echo "verify: $failures check(s) failed"
  exit 1
fi
echo "verify: all checks passed"
