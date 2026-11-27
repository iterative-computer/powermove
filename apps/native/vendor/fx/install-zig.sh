#!/bin/sh
# Powermove: installs the pinned Zig that build.rs uses to build the libfx core.
# Run once by a person or CI; cargo never downloads anything (POWERMOVE.md).
#
#   apps/native/vendor/fx/install-zig.sh
#
# Downloads the tarball toolchain.toml pins for this machine, checks its
# sha256, and unpacks it into $PM_ZIG_HOME (default ~/.cache/powermove/zig) as
# zig-<target>-<version>/. Safe to run again: an existing install whose
# `zig version` matches is left alone.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
pin="$here/toolchain.toml"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-macos ;;
  *) echo "install-zig.sh: no pinned Zig for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
value() { # value <section> <key>
  awk -v section="[$1]" -v key="$2" '
    /^\[/ { inside = ($0 == section) }
    inside && $1 == key { sub(/^[^=]*= */, ""); gsub(/"/, ""); print; exit }
  ' "$pin"
}
version=$(value zig version)
url=$(value "zig.$target" tarball)
sha=$(value "zig.$target" sha256)
[ -n "$version" ] && [ -n "$url" ] && [ -n "$sha" ] || { echo "install-zig.sh: incomplete pin in $pin" >&2; exit 1; }
home=${PM_ZIG_HOME:-$HOME/.cache/powermove/zig}
dest="$home/zig-$target-$version"
if [ -x "$dest/zig" ] && [ "$("$dest/zig" version)" = "$version" ]; then
  echo "Zig $version is already installed at $dest"
  exit 0
fi
mkdir -p "$home"
work=$(mktemp -d "$home/.install.XXXXXX")
trap 'rm -rf "$work"' EXIT
echo "Downloading $url"
curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error -o "$work/zig.tar.xz" "$url"
got=$(shasum -a 256 "$work/zig.tar.xz" | awk '{print $1}')
if [ "$got" != "$sha" ]; then
  echo "install-zig.sh: sha256 mismatch: expected $sha, got $got" >&2
  exit 1
fi
mkdir "$work/unpacked"
tar -xJf "$work/zig.tar.xz" -C "$work/unpacked"
[ -x "$work/unpacked/zig-$target-$version/zig" ] || { echo "install-zig.sh: unexpected tarball layout" >&2; exit 1; }
rm -rf "$dest"
mv "$work/unpacked/zig-$target-$version" "$dest"
echo "Installed Zig $("$dest/zig" version) at $dest"
