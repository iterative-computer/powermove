# Send to Powermove (development plugin)

1. Run `bun install --frozen-lockfile` at the repository root.
2. Run `bun run --cwd packages/figma-plugin build`.
3. In the **Figma desktop app**, choose **Plugins → Development → Import plugin from manifest**.
4. Select `packages/figma-plugin/manifest.json`.
5. In Powermove, choose **File → Connect Figma…**. Enter the displayed code in the plugin and click **Pair**.
6. Select Figma layers and click **Send Selection**. Subsequent sends offer **Update Existing Layers** or **Add as New Copy**.

The development plugin uses `http://localhost:47832`; Powermove binds **127.0.0.1 only**. Figma's [manifest documentation](https://developers.figma.com/docs/plugins/manifest/#networkaccess-networkaccess) explicitly permits HTTP localhost patterns in `devAllowedDomains`. We use `allowedDomains: ["none"]` and only that development exception. This plugin is not published. `enablePrivatePluginApi` enables `figma.fileKey` for stable development-plugin provenance; real Figma import/private-API availability still needs verification. Tokens are stored only in Figma client storage and Powermove memory; restart or **File → Disconnect Figma** revokes pairing. Pairing codes are single-use, expire in five minutes, and lock after ten attempts.

If the Figma desktop runtime blocks local networking, **Copy for Powermove** copies the same document with embedded per-node PNGs. Paste it in Powermove. No remote service or public network access is used. After restarting Powermove, pair again.

The neutral schema and capability check live in `apps/desktop/src/shared/figma/`. Raster fallbacks are exported at 2× only for unsupported scopes. Image-fill bytes are deduplicated by Figma image hash. Circular two-stop gradients use Gradient Ramp; more complex gradients, image fills, inside/outside/dashed/custom-cap strokes and mixed text metrics use per-node PNGs. Plain color text ranges stay editable. Simple clipping and alpha masks use track mattes; nested intersections flatten. Clipboard previews can only safely crop isolated bounds; overlapping scopes are promoted to their smallest enclosing subtree.

Updates retain animation, expressions, user-added effects, timing, parenting and renamed layers. Missing source layers remain in the project and are listed in the import summary. Source metadata is optional and additive, so old `.pmv` projects require no migration.
