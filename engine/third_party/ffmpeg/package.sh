#!/usr/bin/env bash
# Stages Powermove's FFmpeg build for the desktop app.
#
#   engine/third_party/ffmpeg/package.sh [build-out-dir] [stage-dir]
#
# build-out-dir defaults to $PM_FFMPEG_DIR, then build.sh's shared default
# (~/Library/Caches/powermove-engine/ffmpeg/out). stage-dir defaults to
# apps/desktop/vendor/ffmpeg, which electron-builder copies to
# Contents/Resources/ffmpeg (apps/desktop/electron-builder.yml) and which the
# app also uses in development (apps/desktop/src/main/ffmpeg-binary.ts).
#
# Steps:
#   1. verify.sh on the build (license, linkage, required components, checksums);
#      PM_FFMPEG_SKIP_VERIFY=1 skips it
#   2. stage only what ships: bin/ffmpeg, bin/ffprobe, each dylib once under its
#      install name (regular files, no symlinks, no headers or pkg-config files),
#      share/licenses/, manifest.json, and NOTICE.txt (the LGPL notice with the
#      exact source and its checksum)
#   3. check the stage: checksums equal manifest.json, every @rpath dependency
#      resolves inside lib/, nothing links outside lib/ and macOS, ffmpeg and
#      ffprobe run with no DYLD_* help and report the LGPL, the VideoToolbox
#      encoders are present, and libx264 and prores_ks are absent
#   4. replace stage-dir atomically
#
# The staged binaries keep build.sh's ad-hoc signatures. Release packaging
# re-signs everything under Contents/Resources/ffmpeg with the Developer ID and
# the hardened runtime, and apps/desktop/scripts/release-mac.mjs checks it.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
SRC="${1:-${PM_FFMPEG_DIR:-$HOME/Library/Caches/powermove-engine/ffmpeg/out}}"
DEST="${2:-$REPO/apps/desktop/vendor/ffmpeg}"

die() { echo "package.sh: $*" >&2; exit 1; }
[[ -x "$SRC/bin/ffmpeg" && -x "$SRC/bin/ffprobe" && -f "$SRC/manifest.json" ]] \
  || die "no FFmpeg build at $SRC (run engine/third_party/ffmpeg/build.sh, or pass its output directory)"
SRC="$(cd "$SRC" && pwd)"

if [[ "${PM_FFMPEG_SKIP_VERIFY:-0}" != 1 ]]; then
  log="$(mktemp)"
  if ! "$HERE/verify.sh" "$SRC" >"$log" 2>&1; then
    cat "$log" >&2; rm -f "$log"
    die "verify.sh failed for $SRC"
  fi
  echo "verify.sh: $(grep -c '^PASS' "$log") checks passed"
  rm -f "$log"
fi

mkdir -p "$(dirname "$DEST")"
STAGE="$(mktemp -d "$(dirname "$DEST")/.ffmpeg-stage.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/lib" "$STAGE/share"

# Stage from the manifest, so exactly the recorded outputs ship. Prints one
# "<staged path> <sha256>" line per file for the checksum pass below.
python3 -I - "$SRC" "$STAGE" >"$STAGE.sums" <<'PY'
import json, os, shutil, sys
src, stage = sys.argv[1], sys.argv[2]
manifest = json.load(open(os.path.join(src, "manifest.json")))
for out in manifest["outputs"]:
    rel = out["path"]
    if rel.startswith("bin/"):
        target = rel
    elif rel.startswith("lib/") and rel.endswith(".dylib"):
        name = out.get("install_name") or ""
        if not name.startswith("@rpath/"):
            sys.exit(f"{rel}: install name {name!r} is not @rpath/")
        target = "lib/" + name[len("@rpath/"):]
    else:
        continue
    shutil.copy2(os.path.join(src, rel), os.path.join(stage, target))
    print(target, out["sha256"])
shutil.copytree(os.path.join(src, "share", "licenses"), os.path.join(stage, "share", "licenses"))
shutil.copy2(os.path.join(src, "manifest.json"), os.path.join(stage, "manifest.json"))

shipped = [s for s in manifest["sources"] if s.get("role") == "shipped"]
ff = next(s for s in shipped if s["name"] == "ffmpeg")
lines = [
    f"Powermove uses FFmpeg {ff['version']} (https://ffmpeg.org) under the GNU Lesser General",
    "Public License, version 2.1 or later. The FFmpeg libraries in lib/ are dynamically",
    "linked and can be replaced.",
    "",
    f"FFmpeg source: {ff['url']}",
    f"SHA-256: {ff['sha256']}",
    "Build script and configuration:",
    "https://github.com/iterative-computer/powermove/tree/main/engine/third_party/ffmpeg",
    "",
    "Components built into these libraries (license texts and patent grants in",
    "share/licenses/):",
    "",
]
for s in shipped:
    lines.append(f"  {s['name']} {s['version']}  {s['license']}")
    lines.append(f"    {s['url']}")
    lines.append(f"    SHA-256 {s['sha256']}")
open(os.path.join(stage, "NOTICE.txt"), "w").write("\n".join(lines) + "\n")
PY

failures=0
fail() { echo "FAIL  $*" >&2; failures=$((failures + 1)); }

while read -r rel sum; do
  got="$(shasum -a 256 "$STAGE/$rel" | cut -d' ' -f1)"
  [[ "$got" == "$sum" ]] || fail "$rel checksum $got, manifest $sum"
done <"$STAGE.sums"
rm -f "$STAGE.sums"

for bin in "$STAGE"/bin/* "$STAGE"/lib/*; do
  rel="${bin#"$STAGE"/}"
  [[ -L "$bin" ]] && fail "$rel is a symlink"
  while read -r dep; do
    case "$dep" in
      @rpath/*) [[ -f "$STAGE/lib/${dep#@rpath/}" ]] || fail "$rel links $dep, missing from lib/" ;;
      /usr/lib/*|/System/Library/*) ;;
      *) fail "$rel links $dep" ;;
    esac
  done < <(otool -L "$bin" | tail -n +2 | awk '{print $1}' | grep -v "^@rpath/$(basename "$bin")$" || true)
done

for tool in ffmpeg ffprobe; do
  license="$(env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH "$STAGE/bin/$tool" -hide_banner -L 2>&1 || true)"
  grep -q "GNU Lesser General Public" <<<"$license" || fail "staged $tool does not run or is not LGPL"
done
encoders="$("$STAGE/bin/ffmpeg" -hide_banner -encoders 2>/dev/null || true)"
for name in h264_videotoolbox prores_videotoolbox hevc_videotoolbox aac pcm_s24le; do
  grep -qE "^ [A-Z.]{6} $name " <<<"$encoders" || fail "encoder $name missing"
done
for name in libx264 prores_ks prores_aw; do
  grep -qE "^ [A-Z.]{6} $name " <<<"$encoders" && fail "encoder $name must not be present"
done

[[ $failures -eq 0 ]] || die "$failures staging checks failed"

rm -rf "$DEST"
mv "$STAGE" "$DEST"
trap - EXIT
echo "Staged $(du -sh "$DEST" | cut -f1) FFmpeg $(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["ffmpeg"]["version"])' "$DEST/manifest.json") at $DEST"
