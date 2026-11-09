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

KTX2/Basis Universal uses the transcoder already supplied by `three` 0.180.0.
The Basis transcoder is Apache-2.0 (Binomial LLC); its notice is in
[`apps/desktop/assets/licenses/basis-universal.txt`](../../apps/desktop/assets/licenses/basis-universal.txt)
and ships beside the renderer assets as `LICENSE-Basis-Universal.txt`.
The Three loader is MIT-licensed.
Sources: [Basis Universal](https://github.com/BinomialLLC/basis_universal),
[Three](https://github.com/mrdoob/three.js/tree/r180).
The tiny KTX2 image contains original generated color quadrants; regenerate it
with `node apps/desktop/scripts/generate-studio-ktx-fixture.mjs` and an installed
Apache-2.0 `basisu` encoder. No sample artwork is redistributed.
