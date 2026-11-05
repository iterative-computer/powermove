# Native distribution plumbing

Current implementation: see [beta3 preparation](beta2-readiness.md#beta3-implementation-preparation).
`pm-update-policy::sparkle` now owns the supported asynchronous public-API bridge
behind `sparkle-macos`; the synchronous `Installer` contract below remains for
older consumers and tests, not the production Sparkle integration route. Policy
readiness now requires `installer_recovery_verified`, not reverse migration.
The owner has ruled out reverse migration and archive Restore UI; older Restore
requirements below do not override that decision. Application termination/import
leases and real signed installer/Keychain/notarization proofs remain required.


Status: Linux-tested code and local packaging commands, **not cutover approval**.
No app was published, signed, submitted for notarization, installed, tagged or
released. No production feed, credential, Keychain item, profile or release
workflow was changed. Configuration ships disabled with no endpoint or key.

Inputs: [S6 migration](s6-migration.md), [OS declarations](os-base.md),
[first launch](first-launch.md), [P0 decisions](p0-decisions.md), D-STATE,
C-MATRIX, W1/keymap, traces, S1/S5, MCP and engine-service contracts. TR source
was read from `feat/tr-prep` at `3473905563a4091201db68ebe6131ee2b55d381f`:
`docs/native/tr.md`, `src/shared/transition.ts` and
`resources/transition/cutover.json` under `apps/desktop`. TR remains a separate
review branch; its readiness is advisory and its cutover configuration is disabled.

## Decisions and the selecting rules

| Decision | Rule selecting it |
| --- | --- |
| Retain Electron/Squirrel for the first full-ZIP transition; use Sparkle 2 afterward | S6 decision; N4 and pragmatic §9 select a maintained installer with signing, replacement and recovery instead of creating a second installer. |
| Pin Sparkle **2.10.1**, published upstream 2026-10-25, and its upstream archive SHA-256 | N1/N4 require a reviewable dependency; verify the artifact before extraction and re-evaluate security maintenance before distribution. |
| Own only policy checking, bounded archive download and an append-only staging journal; Sparkle owns extraction, archive EdDSA/DR verification, helpers and replacement | N4 avoids an unproven custom installer; N7 exposes stable update ids/states; N8 keeps work on Rust workers. |
| Separate signed JSON policy sidecar from Electron YAML and Sparkle XML; pin an independent policy public key in the signed bundle | N4 binds channel/architecture/identity/version/hash before staging. This additional policy does not replace either updater's authenticated native format. |
| Disabled/unset config, fresh complete six-domain delta, state/secrets, verified installed/TR-or-native receipt, restore proof, and owner gate before native feed contact/staging/install | TR notes, S6 and N4; config alone and a renderer `cutoverReady` Boolean cannot authorize cutover. |
| Re-read migration generations after download and after the normal save/quit barrier | N4 prevents an import/edit/sign-out during network or quit from using stale readiness. Host must hold its migration/import leases through the relevant side effect. |
| Keep every partial, committed receipt, torn pending receipt, previous bundle, installer cache and migration checkpoint | N4 archive-never-delete; installer recovery remains available after policy expiry and never bypasses a fresh quit barrier for a new attempt. |
| Reserve four GiB plus twice the compressed archive size before downloading | N1/N4; conservative staging budget, never prune to make room. Sparkle must additionally check actual extraction/install space. |
| Preserve production id `com.zellzoi.powermove`, `.pmv` single-file UTI and `powermove://`; keep dev/beta ids isolated from production associations | OS-BASE, N4/N6 preserve shipped document/auth entrypoints and avoid beta stealing production ownership. |
| Copy the existing accent catalog and legacy icons; add no appearance or copy | Design ownership; pm-design remains authoritative. All output is provisional until Opus review. |
| Hardened runtime with main-process `allow-jit` only; helpers get no new permissions; preserve reviewed Sparkle nested entitlements when re-signing | N8/S1 need signed V8 JIT, while N3 avoids carrying Electron-only executable-memory, DYLD or library-validation bypasses. No broad Keychain grant. |
| First launch always waits for Electron, verifies optional TR evidence and runs the final delta, including direct DMG and interrupted-Squirrel launches | N4/S6; TR records are source evidence, never the native destination migration checkpoint or closure certificate. |
| Leave all existing consumers intact; compile new `update*.rs` through the packaging workspace crate | Ownership and prune rule; no edits to services exports/manifests, shell, vendor, spikes, mac app or release workflows. The follow-up merge-ready rule explicitly permits workspace membership and its lockfile updates. |

Upstream pin evidence: [Sparkle 2.10.1](https://github.com/sparkle-project/Sparkle/releases/tag/2.10.1).
`apps/native/packaging/sparkle-pin.json` records the public artifact URL and SHA-256
from upstream release metadata. No framework binary is committed or downloaded by
the packaging command. `verify-sparkle-archive` hashes an operator-provided archive;
`bundle` checks the extracted framework version and contained symlinks. Version
metadata alone is not an artifact-authentication substitute: verify the archive
first, extract privately, then pass that framework to the builder.

## Local bundle tooling

`apps/native/packaging/package.py` is Python build tooling, never app runtime.
`build-dmg.sh` is a thin DMG entrypoint. Every output destination must be new;
failures retain their output. There is no release/feed/upload/tag command and
nothing is wired to a release workflow. The explicit `notarize` command submits
only when an operator runs it on macOS with an existing Keychain profile. It was
only exercised with a recorded subprocess double here.

Layout:

```text
Powermove.app/Contents/
  Info.plist
  PkgInfo
  MacOS/powermove-native
  Frameworks/Sparkle.framework/       optional, pinned external input
  Resources/
    Assets.xcassets/                  unchanged existing source catalog
    Assets.car                       actool output on macOS
    asset-info.plist                  actool output
    icon.icns, document.icns          existing shipped assets
    update-config.json               disabled unless explicitly configured
```

`bundle` requires an executable Mach-O input. It generates macOS 14.0 metadata,
production/beta/dev identity, version/build, category, accent asset name and
production-only URL/document declarations. `.pmv` keeps
`com.zellzoi.powermove.project`, `public.data`,
`application/vnd.powermove.project`, Editor role and `LSTypeIsPackage=false`.
Beta is `com.zellzoi.powermove.native-beta`; dev retains the existing
`com.powermove.native.dev`. Neither silently registers the production protocol
or document type. Beta project opening can still use the explicit open-file path.

`--layout-only` skips actool on Linux; its output cannot pass distribution signing
because `sign` requires `Assets.car`. The existing accent source matches
pm-design's light/dark values; the builder makes no new colour decision.
Sparkle bootstrap requires both `--sparkle-feed` and `--sparkle-key`.
It writes `SUFeedURL` and `SUPublicEDKey` and disables automatic checks/install;
only the host's gated, save-barrier-aware updater driver may initiate work.
Enabled policy config also requires the framework/feed/key bootstrap.

Local macOS command shapes, **not executed by this lane**:

```sh
python3 apps/native/packaging/package.py verify-sparkle-archive "$SPARKLE_ARCHIVE"
python3 apps/native/packaging/package.py bundle \
  --executable "$NATIVE_BINARY" --destination "$NEW_APP" \
  --channel cutover --version "$BUNDLE_VERSION" --build "$BUILD_NUMBER" \
  --update-config "$BUNDLED_UPDATE_CONFIG" \
  --sparkle-framework "$VERIFIED_SPARKLE_FRAMEWORK" \
  --sparkle-feed "$SPARKLE_APPCAST_HTTPS_URL" --sparkle-key "$SPARKLE_PUBLIC_KEY"
python3 apps/native/packaging/package.py sign "$NEW_APP"
python3 apps/native/packaging/package.py notarize "$NEW_APP"
python3 apps/native/packaging/package.py transition-zip "$NEW_APP" "$NEW_ZIP"
apps/native/packaging/build-dmg.sh "$NEW_APP" "$NEW_DMG"
python3 apps/native/packaging/package.py notarize "$NEW_DMG"
```

These variables are operator inputs, not defaults or release authorization.
`PM_CODESIGN_IDENTITY` selects an existing Developer ID identity at run time;
`PM_NOTARY_KEYCHAIN_PROFILE` selects an existing `notarytool` Keychain profile.
No Apple ID, password, certificate, Team ID or private EdDSA key is embedded.
Signing walks nested Mach-O/code bundles inside out, rejects escaping symlinks,
uses hardened runtime/timestamps, then verifies deep/strict/all architectures.
The notarization command archives an app before submission and staples/validates
the app itself before the transition ZIP is created. DMG notarization separately
staples the image. A DMG's retained sibling `.dmg-staging` contains the `.app` at
volume root. There is no Finder styling, destructive overwrite or forced quit.

The caller must supply a fully linked native binary and reviewed runtime payload
layout. This builder currently copies the executable, Sparkle and listed assets;
it does not discover/copy arbitrary dylibs, V8 snapshots, engine resources, fonts,
agent binaries or extension bundles. Resource inventories and rpath/deployment-
target verification must be supplied by their owners before real packaging.

## Policy and update API

`pm-distribution` is an explicit native workspace member under
`apps/native/packaging`. It imports the two new service source files by path and
reuses the existing `pm-services::updater::TransitionGate`. Its tests build the
actual owned source without changing another lane's `services/src/lib.rs` or
services manifest. Shell can depend on this crate now; alternatively the services
owner can expose the modules later. Existing services dependencies cover the new
modules; Ed25519 and HTTPS implementations live in packaging. The authorized
`apps/native/Cargo.toml`/`Cargo.lock` changes register this crate and add its required
network dependencies; no pre-existing locked package version was changed.

`UpdateClient` validates bundled `UpdateConfig` and constructs the verifier from
its pinned public key. The default JSON extends TR's disabled
`version/enabled/feedURL` shape with `channel/publicKey`. The endpoint is a new
native **policy sidecar**, not TR's existing generic YAML endpoint; the TR file
and its consumers are unchanged. Generic `Transport`/`SignatureVerifier` APIs are
for trusted host capabilities/testing, never renderer or feed-supplied verifiers.

Enabled endpoints and archive URLs must be HTTPS with no credentials, query or
fragment. Legacy `latest-mac.yml`/`beta-mac.yml` policy endpoints are refused.
`HttpsTransport` keeps TLS verification, disables redirects, accepts only HTTP
200, bounds connect/total time, and streams archive bodies. Feed metadata is
limited to 128 KiB; receipt reads to 256 KiB; downloads use a 64 KiB buffer.
The production client never enables loopback HTTP. The local-feed tests supply
a distinct transport and cross actual loopback HTTP sockets.

The JSON envelope has exactly `payload` (a UTF-8 JSON string) and `signature`
(base64 Ed25519). The signature covers the bytes
`b"Powermove Native update policy v1\0" + payload.as_bytes()`.
Verify before parsing the payload; never normalize/reserialize it first.
Unknown or duplicate structured fields are errors. Payload fields are:

| Field | Contract |
| --- | --- |
| `schema` | Exactly 1. |
| `channel` | `cutover` or `beta`, matched to bundled config. |
| `version` | Display SemVer, strictly newer than installed; stable never offers a prerelease. |
| `sparkleVersion` | Optional signed numeric `CFBundleVersion` / appcast `sparkle:version`, independent of display SemVer. New packaging uses this build number; absent legacy fields fall back to `version`. |
| `architecture` | `arm64` or `x86_64`, matched to the running app. |
| `bundleId` | Exact production or beta id for the channel. |
| `minimumMacos` | At least 14 and no newer than this host for an offer. |
| `archiveUrl` | Credential-free HTTPS; separate full archive. |
| `size` | Exact compressed bytes, nonzero and at most 16 GiB. |
| `sha256` | Lowercase 64-hex full archive digest, bound by the signed policy. |
| `sparkleSignature` | Base64-encoded 64-byte archive EdDSA signature; Sparkle must verify it with `SUPublicEDKey`. |
| `expiresUnix` | Trusted host-clock expiry, checked before a new stage/install. |

The policy key and Sparkle archive key are separate roles. Neither private key is
stored, generated or used by these scripts. Key rotation requires a separately
reviewed authenticated update path; the feed cannot supply a replacement pin.
There is no publisher in this lane and no new production endpoint was invented.
The macOS adapter must bind the selected Sparkle appcast enclosure to this exact
version, architecture, archive bytes and signature, not silently choose a newer
appcast item. The extra policy is not a replacement for Sparkle EdDSA or code
signing. No Rust ZIP extractor or app-replacement code exists here.

`StagingStore` holds a process-exclusive file lock in an explicit private root
(one root per offer id). Id = SHA-256 of the exact signed policy envelope.
Its append-only sequence is:

```text
downloading -> downloaded -> staging -> staged -> install_requested -> completed
```

Receipts and archive attempts use create-new files, fsync and directory fsync;
committed receipt gaps/corruption fail explicitly. A torn `.pending` receipt or
partial archive stays intact. Retry downloads create another retained attempt.
Completed bytes are hashed again before stage, recover and install. The gate is
re-read before download, after download, before install and after quit approval.
The trusted clock is also re-read after download and quit so policy expiry during
those operations cannot authorize staging or installation.
Available disk is checked before contacting the archive endpoint.

`Installer::stage` is idempotent by offer id and must verify Sparkle archive
EdDSA, channel/bundle id, Team/DR continuity, all architectures, notarization,
safe extraction, permissions and actual expanded space before returning its
opaque ticket. `request_install` is reachable only after `QuitBarrier::ready`.
That barrier is the native ordinary save/journal/settings/secrets/dirty-window
quit flow, never a forced updater exit. If it declines, the update stays staged.
`Readiness` must read current generations under host-owned import/migration leases;
the returned Boolean structure is not an untrusted authorization token.

Recovery never contacts a feed or asks the app to quit. A crash in `downloading`
permits an explicit fresh download; `downloaded` can be reverified and staged if
still eligible/unexpired. `staging`/`staged` query the installer's existing id.
`install_requested` asks the installer whether it completed, is still pending, or
is safely staged again. Safe staged recovery requires another normal quit
barrier before retry. Recovery of an existing replacement remains available
after policy expiry; expiry must not strand an owned old bundle. No successful
return here asserts physical power-loss durability or a macOS installation pass.

## First-run handoff

`update_handoff::first_run` coordinates `HandoffHost`, which delegates actual work
to the existing first-launch/kernel/migration owners. It waits for Electron
quiescence without taking/removing its lock. TR launch may reuse evidence only
after the host verifies source bundle/version/DR and hash-bound checkpoint; false
evidence selects the direct importer. Both TR and DMG launches always request the
final delta and return explicit `waiting_for_electron`, `needs_secret_consent`,
`needs_recovery`, `blocked` or `ready` states. Ready requires all six owner domains,
state, secrets, restore proof and equal source/checkpoint generations.

No Keychain access or plaintext enters this module. The macOS host must use TR's
`Powermove Native Legacy Handoff` receipts and exact two trusted-application ACLs
for the production and beta designated requirements. Preserve the old
`Powermove Safe Storage`/`Powermove` item; no Team-wide access group, nil ACL,
new replacement legacy password, secret argv, secret log or plaintext manifest.
Skipped/denied/locked/incomplete handoffs retain ciphertext and require the
existing explicit-consent direct importer. Current sign-outs/deletions must be
preserved by reverse migration; old checkpoint secrets must never overwrite them.

## Verification and remaining integration

Linux: **33 Rust distribution/handoff tests, 20 Python packaging tests, and all
75 existing pm-services tests pass**. Strict Clippy for distribution and services,
Rust formatting, shell syntax, Python compilation and staged gitleaks checks pass.
Python tooling supports 3.9+. Test directories are canonicalized for macOS system
symlinks; icon inputs are synthetic and the unchanged accent fixture is vendored
inside the owned test tree. Fixtures contain synthetic payloads and runtime-generated
test signing material only. No consented real data was needed or touched.

Tests cover a real local HTTP feed, signatures/wrong keys/weak keys/tampering,
config/identity/channel/architecture/OS/version/expiry limits, no network behind a
closed gate, download bounds/checksum/interruption/resume, retained partials and
torn receipts, corrupt journals, locks/symlinks, disk refusal, generation changes,
quit cancellation, staged-byte tampering, install-request interruption/recovery,
DMG/TR first-run deltas, missing closure/state/secrets/restore and source waiting.
Packaging commands use recorded subprocess doubles: no codesign, notarytool,
actool, ditto or hdiutil invocation occurred in these Linux tests.

Changes needed outside this lane's ownership:

- Shell dependency/export wiring and worker scheduling using the existing 30 s
  startup / four-hour update schedule; map structured results to the already
  specified update sheet. No new visual interaction is specified here.
- A real Sparkle 2.10.1 custom user-driver adapter implementing `Installer`, exact
  policy/enclosure binding, normal quit barrier, and idempotent recovery queries.
  No production installer double or success stub is supplied. Inspectable Rust
  staging does not mean the windowed application can install updates yet.
- Connect `HandoffHost`/`Readiness` to pm-first-launch, verified TR records, native
  migration/import generation leases, signed identity checks and current-state
  forward/reverse migration. This lane does not claim those proof Booleans are
  already produced by the application.
- Supply the native resource inventory/rpaths and signed two-DR Keychain consumer.
  Pin and provision the independent public keys and dedicated policy/appcast/YAML
  endpoints in a future reviewed signed configuration. Keep disabled until then.

Required macOS proof: both architectures and macOS 14+ deployment targets; actool
and accent in light/dark; app/DMG/ZIP layout; nested hardened-runtime signatures,
V8 JIT and minimal entitlements; same-DR acceptance and wrong-DR rejection;
notarization/stapling/Gatekeeper, offline/translocated/read-only installation;
production/beta LaunchServices and URL auth; signed two-bundle Keychain ACL,
consent/denial/locked Keychain and current-state Restore; Squirrel's native
`update-downloaded` plus ordinary save/quit; ShipIt/Sparkle interruptions before
and after old-bundle acquisition/replacement/relaunch; power loss/disk/permission
failure; retained profiles/checkpoints/owned old bundles; and macOS 13 staying on
the final compatible Electron feed. Existing S6 closure/restore gates remain open.

`DESIGN-TODO(opus)`: review reuse of existing app/document icons and accent catalog
in the generated bundle, plus the neutral, unstyled DMG volume. No Finder layout,
background, window geometry, copy or new update UI was chosen. All resulting
visual output is provisional until the Opus design review.


## Merge-ready verification — 2026-11-01

Workspace/lockfile integration follows the orchestrator's merge-ready follow-up.
Final fetch/merge base: `4204a153dbf8af916ef2b92d7ca01858e363516a`
(`origin/feat/native-engine`); the merge reports already up to date, with no
conflicts. Shared-lockfile inspection found zero removed or upgraded existing
package versions; additions are confined to this package's dependency closure.
Reproduce from the repository root after activating the pinned Rust toolchain:

```sh
cd apps/native
cargo test --locked -p pm-distribution -p pm-services
cargo clippy --locked -p pm-distribution -p pm-services --all-targets --no-deps -- -D warnings
cargo fmt -p pm-distribution -p pm-services --check
cd ../..
python3 -m unittest discover -s apps/native/packaging/tests -p 'test_*.py'
python3 -m py_compile apps/native/packaging/package.py apps/native/packaging/tests/test_packaging.py
sh -n apps/native/packaging/build-dmg.sh
```

Counts: 33 distribution/handoff + 75 existing services + 20 packaging = **128 tests**.
The macOS checks above remain required; Linux doubles do not certify them.

## 2026-11-04 — beta2 resource audit and Sparkle API blocker

See [beta2 readiness](beta2-readiness.md) for the source-based resource/rpath
inventory, read-only per-slice bundle auditor and signed Mac checklist. The tested
Sparkle policy bridge is not a linked production installer: Sparkle 2.10.1 has no
public external-archive adoption method matching the current synchronous
`Installer::stage` contract. The report specifies the required asynchronous
application/distribution change. No signing or distribution action was performed.
