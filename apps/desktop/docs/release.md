# macOS releases

Powermove currently produces Apple Silicon development-distribution artifacts with electron-builder. The package contains the electron-vite `out/` bundles in an ASAR and includes the native macOS haptics module.

## Build

Use bun 1.3 or newer, install the locked dependencies from the repository root, and run the desktop package's lane:

```sh
bun install --frozen-lockfile   # repo root
bun run dist:mac                # repo root, or from apps/desktop
```

Artifacts are written to `apps/desktop/dist/` (`dist/` relative to this package):

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
bun run dist:release
```

This dedicated lane refuses to build when signing or notarization credentials
are incomplete. It enables Developer ID signing, hardened runtime, and
notarization together, then verifies the app with strict `codesign`, Gatekeeper,
and stapler checks and verifies every generated DMG with `hdiutil`.
The notarization ticket is stapled to the enclosed app, not the DMG.
`bun run dist:mac` remains the credential-free, ad-hoc development package.

## Electron fuses

`scripts/after-pack.mjs` applies these fuses after packaging and before a future signing step:

- `RunAsNode`: off
- `EnableNodeOptionsEnvironmentVariable`: off
- `EnableNodeCliInspectArguments`: off
- `EnableEmbeddedAsarIntegrityValidation`: on
- `OnlyLoadAppFromAsar`: on

The last two settings depend on `asar: true` and electron-builder's embedded ASAR integrity metadata. Inspect a packed app at any time with:

```sh
bunx @electron/fuses read --app apps/desktop/dist/mac-arm64/Powermove.app
```

Fuses are one-way release hardening in practice: rebuild from Electron rather than trying to mutate a distributed app.


## Distribution and OTA updates

Releases are GitHub releases on this repository, `iterative-computer/powermove`.
electron-updater reads the release feed (`latest-mac.yml` for stable releases,
`beta-mac.yml` for prereleases) straight from GitHub, so **the repository must
be public for updates to reach user machines**. While it is private, only
clients with a `GH_TOKEN` in their environment can check for updates.
Releases target **Apple Silicon Macs only**.

`.github/workflows/release.yml` runs on `v*` tags. `vX.Y.Z` publishes a stable
release marked latest; `vX.Y.Z-beta.N` publishes a prerelease. Apps built from
a beta version opt into prereleases; stable builds only see stable releases.
The job uses an arm64 macOS runner, installs the workspace with bun
(`bun install --frozen-lockfile`), writes the tag's version into
`apps/desktop/package.json` with `bun pm version`, tests the update/session
lifecycle with the desktop Vitest subset, signs and notarizes the app through
`bun run dist:release`, and verifies the result in `apps/desktop/dist/`.
It selects Xcode 26.3 on the macOS 15 runner because the Icon Composer catalog
requires `actool` 26 or newer.
It uploads the DMG, ZIP, blockmaps, and the channel manifest into a draft
before publishing the complete release with the workflow's own `GITHUB_TOKEN`.
Nothing is published by the packager itself. Never publish a partial release
or replace assets on an existing tag.

### One-time GitHub configuration

Add these Actions secrets to the repository:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE_P12` | Base64-encoded Developer ID Application certificate exported with its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting that P12 |
| `APPLE_API_KEY_P8` | Contents of the App Store Connect notarization API key |
| `APPLE_API_KEY_ID` | API key ID |
| `APPLE_API_ISSUER_ID` | API issuer ID |

An Apple Development certificate cannot replace Developer ID Application.
Enter secret values through GitHub Settings → Secrets and variables → Actions;
do not put credentials in source files, release notes, or app configuration.
The built app reads the public feed without a token.

### Ship a release

Commit and push the intended source first, then tag that exact commit:

```sh
git tag v1.0.0            # stable
git tag v1.0.1-beta.1     # prerelease
git push origin <tag>
```

Increment the version for every release; a beta increments the beta number.
The workflow sets the packaged version; the local development version does not
need to change. A failed publish may leave a draft: inspect and remove that draft
before rerunning. The manual workflow trigger must select a tag, not a branch.

Share the release's DMG. Testers drag Powermove into Applications and launch it.
Packaged beta apps check 30 seconds after startup and every four hours, download
newer beta releases, and notify only after macOS finishes staging the update.
The app menu also has **Check for Updates…**. Updates apply on normal quit, so
the existing editor/session save barrier runs. Powermove never forces a restart.
Development sessions do not check for updates. Downgrades are disabled.

Before inviting testers, install beta.1 on a separate Mac/profile, create and
save a project, publish beta.2, and verify download, normal quit, updated version,
and restored project content. Unit tests and an unsigned local build do not
prove this signed end-to-end update path. Keep the same app ID and Developer ID
signing identity for every release.
