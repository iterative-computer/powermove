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

## Production signing and notarization

Treat hardened runtime, Developer ID signing, and notarization as one release change. Import a Developer ID Application certificate and expose it to electron-builder through CI secrets:

```sh
export CSC_LINK=/secure/path/DeveloperIDApplication.p12
export CSC_KEY_PASSWORD='certificate-password'
```

Also configure one notarization credential set. App Store Connect API keys are preferred:

```sh
export APPLE_API_KEY=/secure/path/AuthKey_ABC123.p8
export APPLE_API_KEY_ID=ABC123
export APPLE_API_ISSUER=00000000-0000-0000-0000-000000000000
```

Alternatively, electron-builder supports `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, and `APPLE_TEAM_ID`. Never commit certificates, passwords, or API keys.

Run:

```sh
npm run dist:release
```

This dedicated lane refuses to build when signing or notarization credentials
are incomplete. It enables Developer ID signing, hardened runtime, and
notarization together, then verifies the app with strict `codesign`, Gatekeeper,
and stapler checks and verifies every generated DMG with `hdiutil` and stapler.
`npm run dist:mac` remains the credential-free, ad-hoc development package.

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
