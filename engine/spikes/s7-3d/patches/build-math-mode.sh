#!/bin/sh
# Builds the S7 tools against wgpu-hal 30.0.1 with a Metal math-mode switch
# (S7_MATH=safe|relaxed|nofast at run time; unset keeps wgpu's default, fast math).
# wgpu 30 has no API for MTLCompileOptions.mathMode; the engine needs one (plan
# section 8, "Metal math mode"). Output: target/math/release/.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
src=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/wgpu-hal-30.0.1 | head -1)
dst="$here/target/patch-wgpu-hal"
rm -rf "$dst"
cp -R "$src" "$dst"
chmod -R u+w "$dst"
patch -s -d "$dst" -p1 < "$here/patches/wgpu-hal-30.0.1-math-mode.patch"
cd "$here"
# The patch is applied only for this build: Cargo.lock and Cargo.toml stay untouched.
cp Cargo.lock target/Cargo.lock.saved
CARGO_TARGET_DIR=target/math cargo build --release --bins --config 'patch.crates-io.wgpu-hal.path="target/patch-wgpu-hal"'
mv target/Cargo.lock.saved Cargo.lock
