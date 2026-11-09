# Studio import dependencies

FBX import uses `ufbx` 0.11.5 (Rust bindings and the bundled ufbx C reader),
by Samuli Raivio. Both are offered as MIT OR Unlicense; Powermove uses the
MIT alternative. The original notice is in
[`engine/hosts/studio-electron/licenses/ufbx.txt`](../../engine/hosts/studio-electron/licenses/ufbx.txt)
and ships as `Resources/studio/LICENSE-ufbx.txt`.

Sources: [Rust bindings](https://github.com/ufbx/ufbx-rust),
[ufbx](https://github.com/ufbx/ufbx). The build uses `cc` (MIT OR Apache-2.0),
`find-msvc-tools` (MIT OR Apache-2.0), and `shlex` (MIT OR Apache-2.0).

The tiny ASCII FBX fixtures are original, deterministic generated test data.
Regenerate them with `node scripts/generate-studio-fbx-fixtures.mjs`.
No external sample models, textures or font files were added.
