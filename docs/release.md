# macOS releases

Powermove currently produces Apple Silicon development-distribution artifacts with electron-builder. The package contains the electron-vite `out/` bundles in an ASAR and has no native Node modules.

## Build

Use Node 22 or newer, install the locked dependencies, and run:

```sh
npm ci
npm run dist:mac
```

Artifacts are written to `dist/`:

- `Powermove-<version>-mac-arm64.dmg`
- `Powermove-<version>-mac-arm64.zip`
- `mac-arm64/Powermove.app`

The current `identity: null` configuration deliberately skips Developer ID signing. The fuse hook refreshes the arm64 executable's ad-hoc signature after modifying it, but the app is not notarized and is not ready for frictionless public distribution. On another Mac, Gatekeeper may require the user to Control-click or right-click Powermove, choose **Open**, then confirm **Open**.

## Production signing and notarization opt-in

Treat hardened runtime, Developer ID signing, and notarization as one release change. Import a Developer ID Application certificate and expose it to electron-builder through CI secrets:

```sh
export CSC_LINK=/secure/path/DeveloperIDApplication.p12
export CSC_KEY_PASSWORD='certificate-password'
```

Then replace the current `mac.identity`/`mac.hardenedRuntime` settings in `electron-builder.yml` with the following. `CSC_LINK` lets electron-builder select the imported identity, so no certificate name needs to be committed.

```yaml
mac:
  hardenedRuntime: true
  notarize: true
  icon: resources/icon.icns
  target:
    - target: dmg
      arch: [arm64]
    - target: zip
      arch: [arm64]
```

Also configure one notarization credential set. App Store Connect API keys are preferred:

```sh
export APPLE_API_KEY=/secure/path/AuthKey_ABC123.p8
export APPLE_API_KEY_ID=ABC123
export APPLE_API_ISSUER=00000000-0000-0000-0000-000000000000
```

Alternatively, electron-builder supports `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, and `APPLE_TEAM_ID`, or a Keychain profile. Never commit certificates, passwords, or API keys. Re-run `npm run dist:mac` after enabling the block, and verify the result with `codesign --verify --deep --strict --verbose=2`, `spctl --assess --type execute --verbose=4`, and `xcrun stapler validate`.

## Electron fuses

`scripts/after-pack.mjs` applies these fuses after packaging and before a future signing step:

- `RunAsNode`: off
- `EnableNodeOptionsEnvironmentVariable`: off
- `EnableNodeCliInspectArguments`: off
- `EnableEmbeddedAsarIntegrityValidation`: on
- `OnlyLoadAppFromAsar`: on

The last two settings depend on `asar: true` and electron-builder's embedded ASAR integrity metadata. Inspect a packed app at any time with:

```sh
npx @electron/fuses read --app dist/mac-arm64/Powermove.app
```

Fuses are one-way release hardening in practice: rebuild from Electron rather than trying to mutate a distributed app.
