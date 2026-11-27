#!/bin/bash
# Builds, bundles, signs, archives and writes the feeds of a Powermove V2
# release. The release workflow and a local dry run use the same steps.
# Nothing here notarizes, tags, uploads or publishes (package.py notarize and
# the workflow's publish job do, after approval).
#
#   release.sh build                 release binaries (app with telemetry and the real Aero core,
#                                    Store helper, shader compiler) in $CARGO_TARGET_DIR/release
#   release.sh app <new-folder>      <folder>/Powermove.app with Sparkle, signed; on a Developer ID
#                                    signature it must pass V1's Squirrel.Mac requirement
#   release.sh archive <app> <new-folder>   Powermove-<v>-mac-arm64.zip (both updaters) and .dmg
#   release.sh feeds <zip> <new-folder>     latest-mac.yml, beta-mac.yml (V1) and appcast.xml (V2)
#
# Inputs (environment):
#   PM_RELEASE_VERSION   x.y.z, newer than every V1 release (1.3.2 is the last)
#   PM_RELEASE_CHANNEL   cutover (default, com.zellzoi.powermove) or dev (local proofs only)
#   PM_CODESIGN_IDENTITY Developer ID Application identity; or PM_RELEASE_ADHOC=1 (dev bundles only)
#   SPARKLE_DIR          verified, unpacked Sparkle archive (release-inputs.sh)
#   PM_SPARKLE_FEED      appcast URL baked into Info.plist (default: the GitHub latest release)
#   PM_SPARKLE_PUBLIC_KEY   SUPublicEDKey (base64 Ed25519 public key)
#   PM_SPARKLE_KEY_FILE  feeds only: the EdDSA private key file (never in the repo); or
#                        PM_RELEASE_EPHEMERAL_KEY=1 (dry runs): a throwaway key signs feeds that
#                        are never published, since the real key lives only in the release environment
#   PM_FFMPEG_DIR PM_TRANSCRIBE_RUNTIME PM_CODEX_VENDOR PM_CLAUDE_RUNTIME   bundled runtimes
#   PM_RELEASE_REPO (default iterative-computer/powermove), PM_RELEASE_NOTES (Markdown file)
#   PM_CARGO             the cargo command (default cargo; a shared Mac passes its slot wrapper)
#   Local proofs only: PM_RELEASE_BUNDLE_ID (a dev bundle's own com.powermove.native.dev.<name>)
#   and PM_RELEASE_PROFILE=debug (bundle debug binaries; the lane fakes need debug assertions)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
native=$(dirname "$here")
package() { python3 "$here/package.py" "$@"; }
need() { for name; do [ -n "${!name:-}" ] || { echo "release: set $name" >&2; exit 1; }; done; }
version() {
  need PM_RELEASE_VERSION
  [[ $PM_RELEASE_VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "release: version must be x.y.z" >&2; exit 1; }
  # V1's updater only moves forward: V2 must be newer than every V1 release.
  python3 -c 'import sys; v=tuple(map(int,sys.argv[1].split("."))); sys.exit(v <= (1,3,2))' "$PM_RELEASE_VERSION" \
    || { echo "release: $PM_RELEASE_VERSION is not newer than V1 1.3.2" >&2; exit 1; }
}
repo=${PM_RELEASE_REPO:-iterative-computer/powermove}
feed=${PM_SPARKLE_FEED:-https://github.com/$repo/releases/latest/download/appcast.xml}
channel=${PM_RELEASE_CHANNEL:-cutover}
release=${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR}/${PM_RELEASE_PROFILE:-release}

case ${1:?command} in
build)
  cd "$native"
  export PM_BUILD_COMMIT=${PM_BUILD_COMMIT:-$(git rev-parse --short=10 HEAD)}
  export CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
  # require-core: the build fails rather than ship a stub Aero core.
  read -r -a cargo <<< "${PM_CARGO:-cargo}"
  "${cargo[@]}" +1.96.1 build --locked --release -p pm-native-app --features telemetry,pm-agents/require-core
  "${cargo[@]}" +1.96.1 build --locked --release -p pm-runtime-v8 --features macos-sandbox --bin pm-store-helper
  "${cargo[@]}" +1.96.1 build --locked --release -p pm-glsl --bin pm-glsl-compiler
  ;;
app)
  version
  need SPARKLE_DIR PM_SPARKLE_PUBLIC_KEY PM_FFMPEG_DIR PM_TRANSCRIBE_RUNTIME PM_CODEX_VENDOR PM_CLAUDE_RUNTIME
  out=${2:?new folder}
  [ -e "$out" ] && { echo "release: $out exists" >&2; exit 1; }
  mkdir -p "$out"
  app="$out/Powermove.app"
  package bundle --executable "$release/powermove-native" --destination "$app" \
    --channel "$channel" --version "$PM_RELEASE_VERSION" --build "$PM_RELEASE_VERSION" \
    --sparkle-framework "$SPARKLE_DIR/Sparkle.framework" \
    --sparkle-feed "$feed" --sparkle-key "$PM_SPARKLE_PUBLIC_KEY" \
    --transcribe-runtime "$PM_TRANSCRIBE_RUNTIME" \
    --claude-runtime "$PM_CLAUDE_RUNTIME" --codex-vendor "$PM_CODEX_VENDOR" \
    ${PM_RELEASE_BUNDLE_ID:+--bundle-id "$PM_RELEASE_BUNDLE_ID"}
  if [ "${PM_RELEASE_ADHOC:-}" = 1 ]; then
    [ "$channel" = dev ] || { echo "release: ad-hoc signing is for dev bundles only" >&2; exit 1; }
    package sign --adhoc "$app"
  else
    need PM_CODESIGN_IDENTITY
    package sign "$app"
    # The exact check Squirrel.Mac makes before it swaps V1 for this app.
    [ "$channel" = cutover ] && package continuity "$app"
  fi
  package symbols --executable "$release/powermove-native" --destination "$out/symbols" \
    --version "$PM_RELEASE_VERSION" --build "$PM_RELEASE_VERSION"
  echo "$app"
  ;;
archive)
  version
  app=${2:?app}; out=${3:?new folder}
  [ -e "$out" ] && { echo "release: $out exists" >&2; exit 1; }
  mkdir -p "$out"
  package transition-zip "$app" "$out/Powermove-$PM_RELEASE_VERSION-mac-arm64.zip"
  package dmg "$app" "$out/Powermove-$PM_RELEASE_VERSION-mac-arm64.dmg"
  ;;
feeds)
  version
  need SPARKLE_DIR
  zip=${2:?zip}; out=${3:?new folder}
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  unzip -p "$zip" Powermove.app/Contents/Info.plist > "$scratch/Info.plist"
  baked=$(/usr/libexec/PlistBuddy -c "Print :SUPublicEDKey" "$scratch/Info.plist")
  if [ "${PM_RELEASE_EPHEMERAL_KEY:-}" = 1 ]; then
    openssl=$(python3 -c "import importlib.util as u,sys; s=u.spec_from_file_location('p',sys.argv[1]); m=u.module_from_spec(s); s.loader.exec_module(m); print(m.openssl())" "$here/package.py")
    "$openssl" genpkey -algorithm ed25519 -outform DER -out "$scratch/key.der"
    (umask 077; tail -c 32 "$scratch/key.der" | base64 > "$scratch/key")
    PM_SPARKLE_KEY_FILE="$scratch/key"
    PM_SPARKLE_PUBLIC_KEY=$("$openssl" pkey -inform DER -in "$scratch/key.der" -pubout -outform DER | tail -c 32 | base64)
    echo "release: dry run, feeds signed with a throwaway key (the app trusts $baked)" >&2
  else
    need PM_SPARKLE_PUBLIC_KEY PM_SPARKLE_KEY_FILE
    # The key that signs must be the key the app trusts.
    [ "$baked" = "$PM_SPARKLE_PUBLIC_KEY" ] || { echo "release: the app embeds another Sparkle key" >&2; exit 1; }
  fi
  notes=()
  [ -n "${PM_RELEASE_NOTES:-}" ] && notes=(--notes "$PM_RELEASE_NOTES")
  [ -n "${PM_RELEASE_DOWNLOAD_BASE:-}" ] && notes+=(--download-base "$PM_RELEASE_DOWNLOAD_BASE")
  package feeds --zip "$zip" --destination "$out" --version "$PM_RELEASE_VERSION" \
    --build "$PM_RELEASE_VERSION" --tag "v$PM_RELEASE_VERSION" --repo "$repo" \
    --sign-update "$SPARKLE_DIR/bin/sign_update" --ed-key-file "$PM_SPARKLE_KEY_FILE" \
    --public-key "$PM_SPARKLE_PUBLIC_KEY" ${notes[@]+"${notes[@]}"}
  ;;
*) echo "release: unknown command $1" >&2; exit 1 ;;
esac
