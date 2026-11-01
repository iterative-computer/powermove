# OS base: shipped desktop declarations and native cutover inputs

Baseline: **`v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`)**, inspected with `git show` from this repository. This is source/distribution research, with a read-only check of public bundle metadata; no build, app launch, release, feed change or upload was performed. No signing-account identifiers, credentials or private telemetry are included.

## Runtime and minimum macOS

| Fact | Evidence | Consequence for native macOS 14+ |
|---|---|---|
| Electron **44.0.0** | [apps/desktop/package.json](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/package.json): `devDependencies.electron`; installed Electron framework also reports `CFBundleVersion=44.0.0` | Baseline is Electron 44, not an older Chromium/macOS floor |
| Effective existing minimum **macOS 13.0** | `electron-builder.yml` has **no** `minimumSystemVersion`/`LSMinimumSystemVersion` override. Read-only installed application `Contents/Info.plist` declares `LSMinimumSystemVersion=13.0` | Native **14.0** is a deliberate floor increase. Keep macOS 13 users on the final compatible Electron release; do not offer an unusable native bundle |
| Distribution target **arm64** | [electron-builder.yml:147–152](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/electron-builder.yml#L147); release script `--mac --arm64` | Tag does not declare x86_64 or universal output. Do not infer an Intel installed population from generic Electron support |
| Bundle id `com.zellzoi.powermove`, product `Powermove` | `electron-builder.yml:1–2` | Preserve cutover identity and designated requirement for TCC/Keychain/LaunchServices; beta uses its separate identity/profile |
| Category `public.app-category.graphics-design` | `electron-builder.yml:130` | Preserve application category |
| Builder **26.15.3**, updater **6.8.3** | Desktop package dev/dependency pins | Frozen distribution behavior for the transition/restore tests |
| Tag/package version discrepancy | Desktop package at the `v1.2.0` tag still declares **1.1.3**; the inspected installed application also reports **1.1.3** | The installed bundle is corroborating Electron/default-plist evidence, **not an independently authenticated v1.2.0 release artifact**. Do not use its version to rewrite the requested tag baseline |

Actual macOS-version distribution is **not established** by source or a local bundle. Feed-log aggregation remains separate input; no population percentages are invented here. macOS 14+ is the native target decision. Below macOS 26, use material fallbacks at the same intended control surfaces; glass availability is not the deployment floor.

## Signing and entitlements

[resources/entitlements.mac.plist:5–10](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/resources/entitlements.mac.plist#L5) declares exactly **three Boolean entitlements**, all `true`:

| Entitlement | Declared scope | Native action |
|---|---|---|
| `com.apple.security.cs.allow-jit` | File supplied as `mac.entitlementsInherit` | Signed V8 JIT must pass its own test; carry only the entitlement actually required |
| `com.apple.security.cs.allow-unsigned-executable-memory` | Same inherited entitlement file | Reassess for the chosen V8 build; source declaration alone does not establish necessity |
| `com.apple.security.cs.allow-dyld-environment-variables` | Same inherited entitlement file | Reassess for native loading/debug/release policy; do not copy solely because Electron used it |

`mac.entitlements` is **not explicitly set** in this config; only `entitlementsInherit` is set (`electron-builder.yml:131`). Therefore these three keys are an exact **repository declaration**, not a complete authenticated dump of every signed main/helper executable's final entitlements. No app-sandbox entitlement or provisioning-profile file is declared in the tag's builder configuration. A signed archive inspection is still needed for exact delivered entitlement sets/designated requirements.

Local config has `identity: null`, `hardenedRuntime: false`; production release script overrides to `Developer ID Application`, `hardenedRuntime=true`, `notarize=true`, then checks deep/strict code signing, Gatekeeper and stapling (`apps/desktop/scripts/release-mac.mjs:120–150`). **Do not mistake the unsigned local defaults for production signing policy.** Native transcription dylibs/koffi are unpacked and signed with the app; `release-mac.mjs:66–107` verifies native addons share its signing team and hardened runtime. No signing identity value or account information is needed in this public inventory.

Cutover must retain the signing identity relationship. The beta's second bundle identity does not automatically inherit Safe Storage Keychain access. Test the transition release's dual-designated-requirement ACL and the skipped-transition Keychain path in signed builds. A source audit cannot grant TCC permissions or prove those gates.

## Privacy/TCC usage descriptions

There are **zero custom TCC usage strings** in the tag's `electron-builder.yml` `extendInfo`; it only adds the document UTI. The Electron-44-based installed application plist contains **five** default usage-description keys:

| Plist key | Observed default text |
|---|---|
| `NSAudioCaptureUsageDescription` | `This app needs access to audio capture` |
| `NSBluetoothAlwaysUsageDescription` | `This app needs access to Bluetooth` |
| `NSBluetoothPeripheralUsageDescription` | `This app needs access to Bluetooth` |
| `NSCameraUsageDescription` | `This app needs access to the camera` |
| `NSMicrophoneUsageDescription` | `This app needs access to the microphone` |

These are bundle declarations, **not evidence that the app exercises every permission**. No custom Photos, Speech Recognition, Screen Capture, Apple Events or Desktop/Documents/Downloads usage-description key was found in the tag builder config or the inspected application plist. TCC grants themselves are OS-owned, not profile JSON to import. Retain bundle id/signing requirement at cutover and test exercised capabilities; choose native usage strings for the capabilities the native app actually requests. Local browser permission decisions (`apps/desktop/src/main/permissions.ts`) are distinct from macOS TCC consent and extension permission approvals.

## URL schemes and document types

| Registration | Exact declaration and source | Native target |
|---|---|---|
| Public URL scheme | `protocols: [{name: Powermove, schemes: [powermove]}]`, `electron-builder.yml:108–114` | Keep `powermove://` deep links, including Cloud auth completion (`src/main/cloud/auth.ts`); preserve state validation and never log callback credentials |
| Developer protocol opt-in | Packaged builds claim protocol; development only with `POWERMOVE_DEV_PROTOCOL=1` (`src/main/index.ts`, builder comment `:108–110`) | Avoid developer/beta installations silently taking production protocol ownership |
| `.pmv` document association | `ext: pmv`, name/description `Powermove Project`, role **Editor**, MIME `application/vnd.powermove.project`, icon `resources/document.icns`, **`isPackage: false`**, `electron-builder.yml:116–123` | Keep ordinary single-file documents, open-file events and document icon |
| Exported UTI | `com.zellzoi.powermove.project`, conforms to **`public.data`**, filename extension `pmv`, same MIME, icon `document.icns`; `electron-builder.yml:135–146` | Preserve existing UTI/document ownership; `.pmv` can be JSON, PMV3 or PMV4 (D-STATE) |
| Internal renderer origin | `app://powermove`, `src/main/index.ts:92,597` | Internal origin, **not another public LaunchServices scheme**. Chromium storage migration must identify this origin; native compat serving uses the new runtime's origin |

Do not reregister `.pmv` as a directory package or replace it with a new extension at cutover. Project locks/copies and rollback-readable saving are migration behavior, not a change to the UTI. Existing user source/output files remain at their chosen destinations.

## Updater/feed/channel baseline

| Item | Shipped source behavior |
|---|---|
| Provider/feed | GitHub provider, public `iterative-computer/powermove`, `releaseType: draft`; [electron-builder.yml:95–104](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/electron-builder.yml#L95). Feed metadata is GitHub release YAML (`latest-mac.yml`, `beta-mac.yml`), not a Sparkle appcast |
| Artifact pattern | `${productName}-${version}-mac-${arch}.${ext}`, ZIP arm64 for updater; DMG separately created by `scripts/create-dmg.mjs` (`electron-builder.yml:106,147–152`) |
| Channel selection | `release-mac.mjs:120–131`: lowercase prerelease prefix from version (`beta`, etc.), otherwise `latest`, passed as `--config.publish.channel`. Comments explicitly pair stable `latest-mac.yml` and prerelease `beta-mac.yml` (beta can fall back to latest) |
| Eligibility in app | `src/main/updates.ts:16–18`: packaged macOS only. `:75–80`: `allowPrerelease` only when app version contains `-beta.`, `allowDowngrade=false`. No user-facing arbitrary channel switch found |
| Download/install | Automatic download; auto-install on ordinary quit; relaunch only when requested by app UI. It uses Electron native updater/Squirrel and the normal save/quit barrier, not direct `quitAndInstall` (`updates.ts:6–8,75–79,96–119`) |
| Polling | First automatic check after **30 seconds**, then **4 hours** (`updates.ts:122–125`) |
| Update signing | No Sparkle `SUPublicEDKey`, EdDSA appcast/signature configuration or Sparkle dependency found in the inspected tag config/update path. Developer ID/notarized bundle signing is a separate mechanism; absence of EdDSA is not absence of all update validation |
| Minimum-OS feed gate | No native-cutover/macOS-14 transition feed gate exists in the shipped `updates.ts` or builder config. Do not assume the required routing already ships |

**D8 disposition:** keep the final Electron-compatible path for macOS 13. Only the transition release offers a separate native-cutover feed, gated on OS support **and completed migration**. Every native first launch still performs the media delta check, including DMG installation. Test Squirrel delivering the native bundle, interruption, preserved save/quit behavior, bundle identity and restore rollback. Post-cutover Sparkle 2 with a new EdDSA appcast/key pair versus a native updater remains a distribution spike; it is not decided by this inventory. No feed or release was changed.

## Verification limits / remaining evidence

Source declarations, public plist facts and code paths were inspected without running Electron. No signed v1.2.0 archive was built/downloaded/authenticated, no update was installed, and no feed telemetry or TCC/Keychain ACL was read. Before distribution, verify exact production main/helper entitlements, deployment targets of bundled native runtimes/dylibs, signed-identity continuity, LaunchServices ownership under beta/cutover, macOS-13 feed retention and macOS-14 transition behavior. Those are acceptance gates; the facts above do not claim they have passed.
