# The shipping release build

How the native engine's shipped artifacts are built, why the release
profile's thin LTO used to fail or silently not apply, and what was verified
on 2026-10-19 (M4 Max, macOS 27, Rust 1.96.1, Electron 44, no compile-cache
wrapper).

## What ships and how it is built

The workspace profile (`engine/Cargo.toml`) is the only release
configuration; no script adds `RUSTFLAGS`:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
debug = "line-tables-only"
```

| Artifact | Crate (type) | Built by | Installed as |
|---|---|---|---|
| Engine processes' addon | `pm-node-addon` (cdylib) over `pm-node` (rlib) | `bun run build:engine` (`apps/desktop/scripts/build-engine-addon.mjs`): `cargo build --release --locked -p pm-node-addon --features node` | `apps/desktop/native/pm_node.node` |
| Main's addon (frame bridge, font registry) | `pm-bridge-addon` (cdylib) over `pm-bridge` (rlib) | the same script, a second invocation: `cargo build --release --locked -p pm-bridge-addon --features node` | `apps/desktop/native/pm_bridge.node`, `engine/target/pm-bridge/pm_bridge.node` |
| Renderer and document-host module | `pm-wasm-module` (cdylib) over `pm-wasm` (rlib) | `bun run build:wasm` (`engine/crates/pm-wasm/scripts/build.mjs`): `cargo build --release --target wasm32-unknown-unknown -p pm-wasm-module`, custom sections stripped | `apps/desktop/src/renderer/public/engine/pm_wasm.wasm` (`out/renderer/engine/` after `bun run build`) |
| Headless CLI, parity runner | `pm-cli` (bin) | `cargo build --release -p pm-cli` (gates, not shipped in the app) | `engine/target/release/pm-cli` |

The scripts source `engine/env.sh` (pinned toolchain, four jobs, kache as
`RUSTC_WRAPPER` when installed). `scripts/release-mac.mjs --with-engine`
builds `pm_wasm.wasm` (before the renderer bundle, checking
`out/renderer/engine/` carries those bytes), runs `build-engine-addon.mjs`,
stages FFmpeg, and after packaging checks that both `.node` files are in
the bundle and export `napi_register_module_v1` and that `app.asar` holds
the module just built. Without `--with-engine` a stray `native/` addon or
`pm_wasm.wasm` is refused. The GitHub release workflow passes
`--with-engine`, installs the pinned toolchain and restores or builds
FFmpeg. The wasm identity workflow builds the module with the same script
on a hosted runner.

Build the two addons in separate cargo invocations: in one, feature
unification compiles pm-bridge's napi glue into `pm_node.node` as well. The
wrappers' `node` feature is opt-in (like pm-node's and pm-bridge's own) so a
workspace-wide `cargo test` or `clippy` never links Node's symbols into test
binaries; without it the wrapper's build script warns that the dylib has no
Node-API exports.

## What was wrong

Every lane before this built with `CARGO_PROFILE_RELEASE_LTO=false` and
blamed kache. Both failures reproduce with no wrapper at all.

### 1. Thin-LTO links of binaries failed (pm-cli, release tests, examples)

```
error: failed to get bitcode from object file for LTO (Can't find section __bitcode)
error: could not compile `pm-cli` (bin "pm-cli")
```

`RUSTC_LOG=rustc_codegen_llvm::back::lto=info` names the object:

```
adding bitcode from pm_media-….pm_media.…-cgu.0.rcgu.o
adding bitcode from crabby_avif.crabby_avif.…-cgu.00.rcgu.o
error: failed to get bitcode from object file for LTO
```

pm-media's build script linked `libcrabby_avif.a` with
`cargo:rustc-link-lib=static=crabby_avif`. CrabbyAvif is a **Rust
staticlib** (built by `pm-media/native/build.sh` with its own cargo): its
members are named like rustc codegen units (`*.rcgu.o`), carry no LLVM
bitcode, and include a private copy of std. A static library is bundled into
the rlib by default, and at a thin-LTO link rustc reads every `*.rcgu.o`
member of every upstream rlib as a Rust module. CrabbyAvif arrived on
2026-10-18 (7711e0d5); every "LTO fails under kache" note is later than it.
Every binary, test and example above pm-media failed this way.

The hybrid `["cdylib", "rlib"]` crates were not the cause: Cargo builds such
a crate with both object code and bitcode when an LTO'd binary depends on
it, and pm-wasm's rlib carried bitcode.

**Fix.** `cargo:rustc-link-lib=static:-bundle=crabby_avif`: the archive stays
out of pm-media's rlib and rustc passes it to the system linker at the final
link, where its members are ordinary objects. Under LTO the program's own std
is internalized, so CrabbyAvif keeps the private std copy its archive
already carried (as it did in non-LTO links for any symbol not already
defined). Its C API owns its allocations, so nothing crosses allocators.

### 2. The addons and the wasm module were never LTO'd

The second report (a 471 KB `libpm_node.dylib` exporting no N-API symbols)
was a build without `--features node`: pm-node's napi glue is behind that
feature, so the cdylib exported nothing and the linker dead-stripped the
rest. With the feature the dylib exports `napi_register_module_v1` with or
without LTO.

But the shipping addon never had LTO either. `cargo build --release -v -p
pm-node --features node` showed:

```
rustc --crate-name pm_node … --crate-type cdylib --crate-type rlib … -C opt-level=3 -C codegen-units=1   (no -C lto)
```

Cargo only passes `-C lto` when *every* crate type of a unit can be LTO'd,
and an rlib cannot (cargo `core/compiler/lto.rs`: "a cdylib/rlib combination
won't allow LTO"). So `pm_node.node`, `pm_bridge.node` and `pm_wasm.wasm`
shipped without cross-crate LTO despite the profile.

**Fix.** pm-node, pm-bridge and pm-wasm are plain rlibs; three cdylib-only
wrapper crates make the artifacts and get `-C lto=thin` (their dependencies
get `-C linker-plugin-lto`, bitcode only):

- `pm-node-addon`: `use pm_node as _;`, `napi_build::setup()` (the macOS
  `-undefined dynamic_lookup`), the `@loader_path/../../ffmpeg/lib` rpath.
- `pm-bridge-addon`: `use pm_bridge as _;`, `napi_build::setup()`; its
  `harness` feature is the S1 measurement build.
- `pm-wasm-module`: `use pm_wasm as _;`. A cdylib exports its dependencies'
  `#[unsafe(no_mangle)]` functions, so the module's exports and imports are
  unchanged (35 exports, the one `pm_expr.pow` import).

napi-rs 3 registers every `#[napi]` item with a constructor in
`__DATA,__mod_init_func` and `napi` exports `napi_register_module_v1`, the
only symbol Node looks up. Both survive LTO and plain rlib linking with no
export list or `-Wl,-u` (checked in release LTO, release without LTO and
debug builds below).

## Verified

Release profile as committed, no `RUSTC_WRAPPER`, `nice -n 15`, four jobs.
"Non-LTO" is the same tree with `CARGO_PROFILE_RELEASE_LTO=false`.

| Check | Result |
|---|---|
| `cargo build --release -p pm-cli` | failed before; links now |
| `nm -gU` of `pm_node.node` | `_napi_register_module_v1` (LTO, non-LTO, debug) |
| `require()` in Node 24 | pm_node: `EngineHost`, `buildId`, `hostNowUs`; pm_bridge: `BridgeHost`, `FontRegistry`, `nowUs`, `ownTeamId`, `publishPixels`, `serviceName`, `takePixels`, `unlinkPixels` (plus the harness exports with `harness`) |
| Plain-Node render smoke (job engine: handshake, session, inline document, one `agentFrames` frame, pixels checked, shutdown) | passes with the LTO, non-LTO and debug addons; JPEG bytes identical to the old hybrid addon's |
| `build-engine-addon.mjs` | protocol `protocol-25292d6fdfffb933`, bridge loads |
| `cargo test --release -p pm-node` (thin LTO) | 148 passed, 0 failed (19 ignored by default), plus the `fonts` and `render_fonts` main-thread tests |
| `cargo test --release -p pm-wasm -p pm-audio` (thin LTO) | 144 passed, 0 failed, 1 ignored by default |
| `pm-cli --help`; `pm-cli parity render --only render/` (newest render capture) | 81 frames, 0 failed; `render/still-photo-avif` (CrabbyAvif) 100.00% exact |
| `pm-cli parity eval` (native) | exit 0, same summary as non-LTO |
| `pm-cli parity eval --wasm --no-native`, LTO and non-LTO modules | 1,815,420 replies each, 0 differ |
| `pm-wasm/scripts/identity.mjs --seeds 20 --layers 60`, LTO module | 636,463 replies, 0 differ, 0 throws |
| e2e with the LTO addons and module (`pm-electron-lock … --workers=1`) | `engine-render-transport` 7/7, `native-engine` 2/2, `native-engine-transport` 4/4, `engine-doc-boot` 1/1 (2 measurement tests skip); `engine-export` fails at its routed-text step identically with the non-LTO addon (the test still expects text layers to route to the legacy exporter) |

Sizes (bytes; `__TEXT` from `size -m`):

| Artifact | Non-LTO | Thin LTO |
|---|---|---|
| `pm_node.node` file / `__TEXT` | 35,632,920 / 27,361,280 | 35,810,536 / 28,164,096 |
| `pm_bridge.node` file / `__TEXT` | 1,332,232 / 901,120 | 1,289,336 / 901,120 |
| `pm-cli` file / `__TEXT` | 32,208,520 / 24,379,392 | 32,369,016 / 25,296,896 |
| `pm_wasm.wasm` stripped / gzip -9 | 3,426,489 / 1,142,172 | 3,766,423 / 1,227,093 |

LTO inlines across crates, so code grows by about 3% natively and 10% in
the module (7% gzipped); line tables dominate the native file sizes.

Timings (same machine, other lanes running; alternating runs):

| Run | Non-LTO | Thin LTO |
|---|---|---|
| `pm-cli parity eval` (native evaluation, CPU bound) | 5.91, 5.79 s | 5.28, 5.33 s (about 9% faster) |
| `pm-cli render` of `render/transforms` at 3840×2160, ProRes 422 HQ, 90 frames | 4.99, 3.59, 4.21 s | 3.60, 3.58, 4.51 s; decoded frames' MD5 identical |
| `parity eval --wasm --no-native` (module in Node) | 20.9 s | 20.1 s |
| Plain-Node one-frame job render | 78–79 ms | 80–86 ms |

GPU-bound export time is unchanged; evaluation-heavy native code gains
about 9%.

## Reproduce

```sh
source engine/env.sh
bun run --cwd apps/desktop build:engine     # both addons, thin LTO, protocol check
bun run --cwd apps/desktop build:wasm       # pm_wasm.wasm, thin LTO
(cd engine && cargo build --release -p pm-cli)
nm -gU apps/desktop/native/pm_node.node | grep napi_register_module_v1
```

Release tests and examples need no `CARGO_PROFILE_RELEASE_LTO` override.

## Gaps

- Thin LTO through kache (the `env.sh` default) was not rebuilt here; the
  failure never depended on it, but the first kache build after this change
  should be checked with the `nm` line above.
- CrabbyAvif still links its own copy of std (separate panic hook and thread
  locals for its internals). Building it from source inside the workspace
  would remove the copy and let it join LTO.
