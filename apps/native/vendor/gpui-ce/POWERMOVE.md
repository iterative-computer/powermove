# Powermove GPUI-CE fork

Upstream: https://github.com/gpui-ce/gpui-ce
Pinned commit: `bc577476a0eb55173ef985585cfb487f75ab1cab`.
License: Apache-2.0, retained in LICENSE.md and crate license/notice files.
Shallow-cloned with `git clone --depth 1`, then removed the nested .git.
Omitted root assets, examples, benches, tooling, CI and upstream lockfile.
Platform sources/manifests are retained where Cargo resolves optional path dependencies;
only the macOS Metal backend is enabled by the native workspace.
The upstream Rust-authored shaders are retained, rather than replacing their pipeline.

Powermove modifications are described below as they are introduced. Upstream source
notices are preserved. No patch has been submitted upstream yet.

References inspected: Frame's excluded vendor workspace/path overrides
(https://github.com/66HEX/frame/blob/master/Cargo.toml), and
https://github.com/luciascarlet/gpui-glass-demo. The latter uses custom WGPU glass;
we use AppKit materials per the native composition contract, with no shader code copied.
