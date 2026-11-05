#!/bin/sh
# Local compile/link/guard check. No check for updates, signing or installation.
set -eu
if [ "$(uname -s)" != Darwin ]; then
    echo 'BLOCKED: Sparkle binding compilation/linking requires macOS' >&2
    exit 2
fi
: "${PM_SPARKLE_FRAMEWORK_DIR:?set the parent of the verified Sparkle.framework}"
: "${PM_SPARKLE_ARCHIVE:?set the pinned upstream Sparkle archive}"
PM_BINDING_NATIVE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
python3 "$PM_BINDING_NATIVE/packaging/package.py" verify-sparkle-archive "$PM_SPARKLE_ARCHIVE"
DYLD_FRAMEWORK_PATH="$PM_SPARKLE_FRAMEWORK_DIR${DYLD_FRAMEWORK_PATH:+:$DYLD_FRAMEWORK_PATH}"
export DYLD_FRAMEWORK_PATH
cargo run --locked --manifest-path "$PM_BINDING_NATIVE/Cargo.toml" \
    -p pm-update-policy --features macos-sparkle --example sparkle-link-check
