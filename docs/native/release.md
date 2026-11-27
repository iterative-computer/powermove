# Releasing Powermove V2

One manual workflow, `.github/workflows/release-v2.yml` (Actions › Release Powermove V2),
builds, signs, notarizes and publishes a V2 release. Its steps live in
`apps/native/packaging/release.sh` and `package.py`, so a local dry run runs the same code.

A release is one GitHub release `v<version>` marked **Latest**, with five assets:

| Asset | Read by |
| --- | --- |
| `Powermove-<v>-mac-arm64.dmg` | people downloading V2 |
| `Powermove-<v>-mac-arm64.zip` | both updaters (one signed, notarized, stapled app) |
| `latest-mac.yml`, `beta-mac.yml` | V1's electron-updater 6.8.3 (stable and beta V1 users) |
| `appcast.xml` | V2's Sparkle 2.10.1, at `https://github.com/iterative-computer/powermove/releases/latest/download/appcast.xml` |

The appcast lives on the GitHub release because the same publish step that ships the ZIP
ships the feed, with no second host to deploy or keep in step. Its download links point at
the tagged release, so they never change.

**V1 → V2.** V1 checks 30 seconds after launch and every 4 hours, downloads the ZIP in the
background (SHA-512 checked) and Squirrel.Mac swaps the app in place when the person quits.
Squirrel only accepts a bundle with the same identifier (`com.zellzoi.powermove`) that
satisfies V1's designated requirement (Developer ID, Team `8JTS5XWJJN`;
`packaging/v1-designated-requirement.txt`, read from the signed V1 1.3.2). The workflow runs
that exact check (`package.py continuity`) before and after notarization. V1 users on
macOS 13 are not offered V2 (`minimumSystemVersion: 23.0.0`, the Darwin version of macOS 14).
On its first launch V2 finds `~/Library/Application Support/Powermove` and no library of its
own, so it runs the migration (Bring It In) into its own profile before the editor opens.

**V2 → V2.** Sparkle checks once a day, shows its update window with the notes, verifies the
EdDSA signature of the appcast and of the archive against `SUPublicEDKey` in Info.plist, and
replaces the app through the app's normal quit (unsaved projects still ask). Development,
lane and profile launches never use the public feed (`crates/app/src/updates.rs`
`feed_allowed`, and `package.py feed_url` for dev bundles).

## One-time setup

1. **Sparkle keys** (on the owner's Mac, once). Download Sparkle 2.10.1, check it, generate:

   ```sh
   curl -LO https://github.com/sparkle-project/Sparkle/releases/download/2.10.1/Sparkle-2.10.1.tar.xz
   shasum -a 256 Sparkle-2.10.1.tar.xz   # 1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27
   mkdir Sparkle && tar -xJf Sparkle-2.10.1.tar.xz -C Sparkle
   ./Sparkle/bin/generate_keys --account powermove-v2             # prints the public key
   ./Sparkle/bin/generate_keys --account powermove-v2 -x sparkle-private.txt
   ```

   `generate_keys` keeps the private key in your login Keychain. Put the public key in the
   repository **variable** `SPARKLE_ED_PUBLIC_KEY`, the contents of `sparkle-private.txt` in
   the `release` environment **secret** `SPARKLE_ED_PRIVATE_KEY`, keep one offline backup,
   then delete the file. Without this key no V2 can ever update itself again.
2. **Environment** `release` (Settings › Environments): required reviewer: the owner;
   deployment branches: the release branch only. Environment secrets:
   `APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID` (the App Store Connect key
   V1 uses) and `SPARKLE_ED_PRIVATE_KEY`.
3. **Repository secrets** (V1's, reused as they are): `APPLE_CERTIFICATE_P12` (base64 of the
   Developer ID Application .p12) and `APPLE_CERTIFICATE_PASSWORD`.
4. Actions enabled for the repository.

## Cutting a release

1. **Dry run.** Run workflow on the release branch with the version (newer than 1.3.2,
   e.g. `2.0.0`), the notes, and *Dry run* checked. It builds (the first run also builds the
   V8 sandbox library from source and caches it), signs with the Developer ID, checks V1's
   requirement and keeps `signed-app-<v>`, `symbols-<v>` and `dry-run-<v>` (DMG and feeds signed
   with a throwaway key, checked with electron-updater 6.8.3's own parser). Nothing is published.
2. Install the dry-run DMG on a spare account and open it (not notarized: Control-click › Open).
   Check that Powermove › Check for Updates… opens Sparkle's window: this is the first launch
   of a Developer ID build, where the hardened runtime loads Sparkle and the FFmpeg libraries.
3. **Release.** Run workflow again with the same version and notes and *Dry run* unchecked.
   Approve the `release` deployment when GitHub asks. The job notarizes and staples the app
   and the DMG, writes and signs the feeds, checks them with electron-updater, then publishes
   `v<version>` as Latest (draft first, every asset uploaded, then published).
4. Within a few hours V1 users have it waiting for their next quit; V2 users within a day.

## Cautions

- A later V1 1.x release must not be marked Latest: it would take over both feeds.
- V1's `release.yml` (on `main`) builds Electron for any pushed `v*` tag. This workflow tags
  through the GitHub API, which starts no workflow; never push a V2 tag by hand while `main`
  carries `release.yml`, and retire it when V2 lands on `main`.
- Versions only go up: both updaters ignore an older or equal version.

## Local dry run

Ad-hoc signing on a dev bundle with a loopback feed, a throwaway key in a temporary folder:

```sh
source ~/Developer/pm-durable/env-native.sh
eval "$(apps/native/packaging/release-inputs.sh /private/tmp/pm-release/inputs | sed 's/^/export /')"
PM_CARGO="slot cargo" apps/native/packaging/release.sh build
export PM_RELEASE_VERSION=2.0.0 PM_RELEASE_CHANNEL=dev PM_RELEASE_ADHOC=1 \
  PM_SPARKLE_FEED=http://127.0.0.1:8765/appcast.xml PM_SPARKLE_PUBLIC_KEY=<throwaway public key>
apps/native/packaging/release.sh app /private/tmp/pm-release/app.noindex
apps/native/packaging/release.sh archive /private/tmp/pm-release/app.noindex/Powermove.app /private/tmp/pm-release/out
PM_RELEASE_EPHEMERAL_KEY=1 apps/native/packaging/release.sh feeds /private/tmp/pm-release/out/Powermove-2.0.0-mac-arm64.zip /private/tmp/pm-release/out/feeds
python3 -m unittest discover -s apps/native/packaging/tests -p 'test_*.py'
```

## What is proven, and what is not yet

Proven locally (2026-11-27, ad-hoc, loopback, throwaway key): the release binary bundles,
signs, archives and gets feeds; electron-updater 6.8.3's GitHub provider offers the ZIP to
stable and beta V1 on macOS 14+, skips macOS 13, and its SHA-512 check rejects a tampered ZIP;
V1 1.3.2's own app satisfies the recorded requirement; Sparkle updated a V2 build 2.0.0 to
2.0.1 in place on a normal quit and refused an edited (unsigned) appcast.

Not proven here: the Squirrel.Mac swap from a running V1 (a production-id bundle on the owner's
Mac would register `powermove://` and `.pmv` with LaunchServices), Developer ID signing,
notarization and the hardened runtime (the first dry run and step 2 above prove those), and
the first V2 launch migrating a real V1 profile after the swap (the code path is
`first_run::Plan::resolve` → `pm_first_launch::detect` → `Launch::Migrate`). V1 builds older
than electron-updater 6.8.3 may not honour `minimumSystemVersion`. Keychain items V1 created
are readable by V2 only if their access control trusts V1's requirement, which V2 satisfies;
not checked (Keychain rule). What's New has no release-notes source in shipped builds.
