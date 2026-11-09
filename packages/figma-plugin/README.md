# Send to Powermove

You do not need the repository or a terminal. The Powermove app includes the plugin.

1. In Powermove, choose **File → Connect Figma…** and click **Copy Plugin Path**. **Show in Finder** also reveals the file.
2. In the Figma desktop app, choose **Plugins → Development → Import plugin from manifest…**.
3. In the file picker, press **⌘⇧G**, paste the copied path, press Return, and open `manifest.json`. Add the plugin once.
4. Run **Send to Powermove** in Figma and click **Connect**. Compare the six-digit code in both apps, then click **Allow** in Powermove.
5. Open a composition in Powermove. Select layers in Figma and click **Send … Layers**.

Both apps remember the connection after restarts. The imported plugin lives in a stable **Figma Plugin** folder under Powermove's app data; opening Connect Figma refreshes that copy from the installed app after an update. **Disconnect** in the sheet or **File → Disconnect Figma** revokes pairing and stops the local listener. Import notices offer **Connect Figma** when the plugin can improve clipboard results.

If connecting fails, the plugin offers **Copy for Powermove**. Paste the copied selection in Powermove. An unavailable font uses the editor's fallback font; auto-width text stays on one line. Re-sending offers **Update Existing Layers** or **Add as New Copy**. Updates retain animation, expressions, user effects, timing, parenting and renamed layers. Missing layers are retained and listed.

## Connection and file identity

The listener binds **127.0.0.1 only**. The plugin uses `http://localhost:47832`, explicitly listed in `allowedDomains` with a reasoning sentence. Figma documents localhost support in [Plugin Manifest](https://developers.figma.com/docs/plugins/manifest/) and [Version 1, Update 78](https://developers.figma.com/docs/plugins/updates/2023/09/21/version-1-update-78/). No public relay is involved.

New connections require the app's Connect Figma sheet to be open. One request waits for up to a minute; Don't Allow starts a short cooldown. The iframe's `null` origin is never treated as proof of permission. `/status` returns reachability, token validity, composition availability and protocol version, without tokens or approval codes. Tokens are random; Figma stores the token in client storage, while Powermove stores only its SHA-256 hash in a private `figma-link.json` file outside project files. Background tests leave the listener off unless they explicitly request restart testing.

Each document gets a random `powermove-file-id` through the public root plugin-data API. Node IDs and that identity remain stable across sends and restarts. No private Figma API is needed. Imports from earlier development commits that used a private Figma file key may need importing once again; no shipped-project migration is involved.

The development manifest uses the stable local ID `powermove-local`. Real Figma accepts a manifest without an ID but rejects client storage in that plugin, so the ID is necessary to remember pairing. The desktop host forwards sandbox messages from its top frame, through an intermediate wrapper; the UI accepts that sender only with a Figma origin, alongside the immediate parent used by other hosts.

## Building and packaging

For contributors: `bun run --cwd packages/figma-plugin build` produces `dist/code.js` and `dist/ui.html`. The UI bundles the shared Powermove design tokens and follows the Figma host’s light or dark theme. The desktop Vite build and electron-builder's before-pack hook both build the plugin. `extraResources` ships only the manifest, UI and compiled code. The main process resolves development and packaged resource paths separately and checks a content fingerprint before refreshing the stable copy.

## Publishing to the Figma Community

This repository does not publish the plugin automatically.

- Have the owner create the plugin entry in Figma and replace the development ID in `manifest.json` with its assigned Community ID. Changing IDs changes the plugin's storage namespace, so verify document identity and pair again for the published plugin.
- Keep `http://localhost:47832` in `allowedDomains` and retain the plain reasoning sentence. Keep private APIs disabled.
- Verify the installed app workflow, localhost resolution/CSP, public document identity, PNG export and visual fidelity in a new draft in real Figma; repeat approval, send, re-sync and both app restarts.
- Check the packaged plugin files and refresh behavior with an installed app update.
- Publish only with the owner's explicit approval. Set the single `FIGMA_COMMUNITY_URL` constant in `apps/desktop/src/shared/figma-link.ts` to the resulting listing URL; the app will offer **Open in Figma** instead of development import steps.
