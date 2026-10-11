#!/usr/bin/env bash
# Build spike S5: the wasm module and bindings (pkg/), and the native runner.
#
#   engine/spikes/s5-wasm/scripts/build.sh
#
# Uses the pinned toolchain (engine/rust-toolchain.toml via engine/env.sh) and
# a wasm-bindgen CLI pinned to the crate's version, installed into .tools/.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$here/../../env.sh"
cd "$here"

bindgen_version="$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' Cargo.toml)"
bindgen="$here/.tools/bin/wasm-bindgen"
if [[ ! -x "$bindgen" ]] || [[ "$("$bindgen" --version)" != "wasm-bindgen $bindgen_version" ]]; then
  cargo install --locked --root "$here/.tools" wasm-bindgen-cli --version "$bindgen_version"
fi

cargo build --release --target wasm32-unknown-unknown -p s5-wasm
rm -rf pkg
"$bindgen" --target web --out-dir pkg target/wasm32-unknown-unknown/release/s5_wasm.wasm

cargo build --release -p s5-native
cargo build --release -p s5-native --features system-math --target-dir target/system-math
echo "built pkg/ ($(wc -c < pkg/s5_wasm_bg.wasm | tr -d ' ') bytes), target/release/s5-native, target/system-math/release/s5-native"
