#!/usr/bin/env python3
"""Local distribution operations only. No upload, feed publish, release or tag API.

Python is build tooling, never bundled. macOS commands are injectable for Linux
contract tests. Destination paths must be new; existing bundles are never removed.
The release workflow (.github/workflows/release-v2.yml) and release.sh drive it;
docs/native/release.md is the operator's guide.
"""
import argparse
import base64
from datetime import datetime, timezone
from email.utils import format_datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
from urllib.parse import urlsplit
from xml.sax.saxutils import escape

HERE = Path(__file__).resolve().parent
NATIVE = HERE.parent
ROOT = NATIVE.parents[1]
IDS = {
    "cutover": "com.zellzoi.powermove",
    "beta": "com.zellzoi.powermove.native-beta",
    "dev": "com.powermove.native.dev",
}
UTI = "com.zellzoi.powermove.project"
MACHO = {bytes.fromhex(m) for m in (
    "feedface", "cefaedfe", "feedfacf", "cffaedfe", "cafebabe", "bebafeca", "cafebabf", "bfbafeca"
)}


def run(args):
    subprocess.run([str(arg) for arg in args], check=True)


def macos():
    if sys.platform != "darwin":
        raise ValueError("this operation requires macOS; use Linux contract tests")


def https(value):
    parsed = urlsplit(value)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username is not None
            or parsed.password is not None or parsed.query or parsed.fragment):
        raise ValueError("expected a credential-free HTTPS URL without query/fragment")
    # Reject malformed ports, including nonnumeric values.
    _ = parsed.port
    return value


def public_key(value):
    if len(base64.b64decode(value, validate=True)) != 32:
        raise ValueError("expected an Ed25519 public key")
    return value


LOOPBACK = {"127.0.0.1", "::1"}


def feed_url(value, channel):
    """A shipping channel takes a credential-free HTTPS appcast. A dev bundle may
    only name a loopback appcast (a local proof), so dev and lane builds never
    reach the real feed; the app enforces the same rule (crates/app updates.rs)."""
    if channel == "dev":
        parsed = urlsplit(value)
        if (parsed.scheme not in ("http", "https") or parsed.hostname not in LOOPBACK
                or parsed.username is not None or parsed.password is not None):
            raise ValueError("a dev bundle may only use a loopback appcast")
        return value
    https(value)
    if urlsplit(value).path.endswith(("latest-mac.yml", "beta-mac.yml")):
        raise ValueError("the Electron feed is not a Sparkle appcast")
    return value


def info(channel, version, build, sparkle_feed=None, sparkle_key=None, bundle_id=None):
    if channel not in IDS or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("explicit channel and numeric macOS bundle version required")
    if not re.fullmatch(r"[1-9]\d*(?:\.\d+){0,2}", build):
        raise ValueError("invalid bundle build number")
    if bool(sparkle_feed) != bool(sparkle_key):
        raise ValueError("Sparkle feed and public key must be provided together")
    # A dev bundle may take a lane's own id (com.powermove.native.dev.<lane>).
    if bundle_id and (channel != "dev" or not re.fullmatch(r"com\.powermove\.native\.dev(?:\.[A-Za-z0-9-]+)+", bundle_id)):
        raise ValueError("only a dev bundle takes its own com.powermove.native.dev.<name> id")
    value = {
        "CFBundleName": "Powermove", "CFBundleDisplayName": "Powermove",
        "CFBundleIdentifier": bundle_id or IDS[channel], "CFBundleExecutable": EXECUTABLE,
        "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version,
        "CFBundleVersion": build, "LSMinimumSystemVersion": "14.0",
        # The system allocator otherwise keeps freed large blocks (V8, codecs)
        # resident for minutes; see apps/native/crates/app/src/main.rs.
        "LSEnvironment": {"MallocLargeCache": "0"},
        "LSApplicationCategoryType": "public.app-category.graphics-design",
        "NSAccentColorName": "AccentColor", "NSHighResolutionCapable": True,
        "NSPrincipalClass": "NSApplication", "CFBundleIconFile": "icon.icns",
        "CFBundleIconName": "Powermove",
    }
    # Beta/dev remain isolated; only production takes the shipped associations.
    if channel == "cutover":
        value["CFBundleURLTypes"] = [{"CFBundleURLName": "Powermove", "CFBundleURLSchemes": ["powermove"]}]
        value["CFBundleDocumentTypes"] = [{
            "CFBundleTypeName": "Powermove Project", "CFBundleTypeRole": "Editor",
            "CFBundleTypeExtensions": ["pmv"], "LSItemContentTypes": [UTI],
            "CFBundleTypeMIMETypes": ["application/vnd.powermove.project"],
            "CFBundleTypeIconFile": "document.icns", "LSTypeIsPackage": False,
        }]
        value["UTExportedTypeDeclarations"] = [{
            "UTTypeIdentifier": UTI, "UTTypeConformsTo": ["public.data"],
            "UTTypeDescription": "Powermove Project", "UTTypeIconFile": "document.icns",
            "UTTypeTagSpecification": {"public.filename-extension": ["pmv"],
                                       "public.mime-type": "application/vnd.powermove.project"},
        }]
    if sparkle_feed:
        # Sparkle checks once a day without asking first and shows its update
        # window; it never installs silently. Every archive and the appcast
        # itself must carry an EdDSA signature from the pinned key, checked
        # before extraction. Replacement waits for the app's normal quit.
        value.update(SUFeedURL=feed_url(sparkle_feed, channel), SUPublicEDKey=public_key(sparkle_key),
                     SUEnableAutomaticChecks=True, SUAutomaticallyUpdate=False,
                     SUVerifyUpdateBeforeExtraction=True, SURequireSignedFeed=True)
    return value


def rpaths(binary):
    """LC_RPATH entries of a Mach-O file (otool -l)."""
    text = subprocess.run(["otool", "-l", str(binary)], check=True, capture_output=True, text=True).stdout
    return sorted(set(re.findall(r"^\s*path (.+?) \(offset \d+\)$", text, re.M)))


def new_destination(path):
    path = Path(path).absolute()
    if path.exists() or path.is_symlink():
        raise ValueError("destination exists; preserve it and choose a new destination")
    for parent in path.parents:
        if parent.is_symlink():
            raise ValueError("destination has a linked parent")
    return path


def native_file(path):
    path = Path(path)
    if not path.is_file() or path.is_symlink() or not os.access(path, os.X_OK):
        raise ValueError("expected an executable native binary")
    with path.open("rb") as binary:
        if binary.read(4) not in MACHO:
            raise ValueError("expected a Mach-O native executable")
    return path


# The bundle's executable keeps V1's name. Squirrel.Mac (V1's updater) installs
# the update as "<CFBundleExecutable>.app", so any other name would move a V1
# user's /Applications/Powermove.app to a new path on the V2 update.
EXECUTABLE = "Powermove"
HELPERS = ("pm-store-helper", "pm-glsl-compiler")


# Bundle resources the app reads by name (bundle-dev.sh copies the same set).
RESOURCES = {
    "little-victory-deep.wav": "bundle/little-victory-deep.wav",
    "Credits.rtf": "bundle/Credits.rtf",
    "Distortion Waves License.txt": "bundle/Distortion Waves License.txt",
    "Pierre Icons License.txt": "crates/shell/icons/pierre/LICENSE",
}


def bundle(executable, destination, channel, version, build,
           sparkle_framework=None, sparkle_feed=None, sparkle_key=None, compile_assets=True,
           glsl_compiler=None, claude_runtime=None, codex_vendor=None, transcribe_runtime=None,
           bundle_id=None):
    metadata = info(channel, version, build, sparkle_feed, sparkle_key, bundle_id)
    if bool(sparkle_framework) != bool(sparkle_feed):
        raise ValueError("Sparkle needs its framework, feed and key together")
    executable = native_file(executable)
    destination = new_destination(destination)
    if destination.suffix != ".app":
        raise ValueError("bundle destination must end in .app")
    if compile_assets:
        macos()
    if sparkle_framework:
        framework = Path(sparkle_framework)
        if framework.name != "Sparkle.framework" or not framework.is_dir() or framework.is_symlink():
            raise ValueError("expected a reviewed Sparkle.framework")
        pin = json.loads((HERE / "sparkle-pin.json").read_text())
        framework_info = (framework / "Resources/Info.plist").resolve(strict=True)
        if not framework_info.is_relative_to(framework.resolve(strict=True)):
            raise ValueError("framework metadata escapes its root")
        if plistlib.loads(framework_info.read_bytes())["CFBundleShortVersionString"] != pin["version"]:
            raise ValueError("Sparkle framework does not match the reviewed version pin")
        # Also reject external directory/file aliases before copying nested code.
        nested_code(framework)
    contents = destination / "Contents"
    resources = contents / "Resources"
    (contents / "MacOS").mkdir(parents=True)
    resources.mkdir()
    shutil.copy2(executable, contents / "MacOS" / EXECUTABLE)
    # Worker executables the app starts by path: the confined Store helper and
    # the killable shader compiler. They sit beside the build's main binary
    # unless the compiler is given from another build.
    helpers = {helper: executable.with_name(helper) for helper in HELPERS}
    if glsl_compiler:
        helpers["pm-glsl-compiler"] = Path(glsl_compiler)
    for helper, source in helpers.items():
        if not source.is_file():
            raise ValueError(f"{helper} is missing at {source}")
        shutil.copy2(native_file(source), contents / "MacOS" / helper)
    # The media service resolves these beside the main executable. Their
    # build-tree rpath points at ../lib; distribution dylibs live in Frameworks.
    ffmpeg_dir = Path(os.environ.get("PM_FFMPEG_DIR",
        Path.home() / "Library/Caches/powermove-engine/ffmpeg/out"))
    for tool in ("ffmpeg", "ffprobe"):
        target = contents / "MacOS" / tool
        shutil.copy2(native_file(ffmpeg_dir / "bin" / tool), target)
        if compile_assets:
            run(["install_name_tool", "-rpath", "@executable_path/../lib",
                 "@executable_path/../Frameworks", target])
    # The app and the tools link FFmpeg's libraries by @rpath (libavformat.63.dylib
    # and so on): each goes into Frameworks under that name, and the executable
    # loses the build machine's absolute rpath so it loads only what it carries.
    frameworks = contents / "Frameworks"
    frameworks.mkdir()
    libraries = sorted(p for p in (ffmpeg_dir / "lib").glob("lib*.dylib")
                       if re.fullmatch(r"lib\w+\.\d+\.dylib", p.name))
    if not libraries:
        raise ValueError("FFmpeg libraries are missing from PM_FFMPEG_DIR/lib")
    for library in libraries:
        shutil.copyfile(library.resolve(strict=True), frameworks / library.name)
        (frameworks / library.name).chmod(0o755)
    if compile_assets:
        for rpath in rpaths(contents / "MacOS" / EXECUTABLE):
            if not rpath.startswith(("@executable_path", "@loader_path")):
                run(["install_name_tool", "-delete_rpath", rpath, contents / "MacOS" / EXECUTABLE])
    # sign() discovers both Mach-O tools and signs them with the other helpers.
    # The layout provider discovery reads: <name>/bin/<name> under Resources.
    if claude_runtime:
        (resources / "claude" / "bin").mkdir(parents=True)
        shutil.copy2(native_file(claude_runtime), resources / "claude" / "bin" / "claude")
    if codex_vendor:
        codex_vendor = Path(codex_vendor)
        native_file(codex_vendor / "bin" / "codex")
        shutil.copytree(codex_vendor, resources / "codex", symlinks=True)
    if transcribe_runtime:
        # Speech to text: transcribe.cpp's contract and dylibs (signed with the app).
        runtime = Path(transcribe_runtime)
        if not (runtime / "contract.json").is_file():
            raise ValueError("transcribe runtime needs contract.json")
        (resources / "transcribe").mkdir()
        for item in [runtime / "contract.json", *sorted(runtime.glob("*.dylib"))]:
            shutil.copy2(item, resources / "transcribe" / item.name)
    for name, source in RESOURCES.items():
        source = NATIVE / source
        if source.is_file():
            shutil.copy2(source, resources / name)
    with (contents / "Info.plist").open("wb") as file:
        plistlib.dump(metadata, file, sort_keys=True)
    (contents / "PkgInfo").write_bytes(b"APPL????")
    for name in ("icon.icns", "document.icns"):
        shutil.copy2(NATIVE / "bundle" / name, resources / name)
    # Source catalog is owned by pm-design/Opus; never generate new colour values.
    shutil.copytree(NATIVE / "bundle" / "Assets.xcassets", resources / "Assets.xcassets")
    if compile_assets:
        command = ["xcrun", "actool", "--compile", resources, "--platform", "macosx",
                   "--minimum-deployment-target", "14.0", "--accent-color", "AccentColor",
                   "--output-partial-info-plist", resources / "asset-info.plist",
                   resources / "Assets.xcassets"]
        icon_bundle = NATIVE / "bundle" / "Powermove.icon"
        if icon_bundle.is_dir():
            command.extend(["--app-icon", "Powermove", icon_bundle])
        run(command)
    if sparkle_framework:
        shutil.copytree(framework, contents / "Frameworks" / "Sparkle.framework", symlinks=True)
    return destination


def nested_code(app):
    """Inside-out signing, without --deep signing or traversing linked trees."""
    app = Path(app).resolve(strict=True)
    targets = []
    for directory, dirs, files in os.walk(app, followlinks=False):
        for name in dirs:
            path = Path(directory) / name
            if path.is_symlink() and not path.resolve(strict=True).is_relative_to(app):
                raise ValueError("code symlink escapes the bundle")
        dirs[:] = [name for name in dirs if not (Path(directory) / name).is_symlink()]
        for name in files:
            path = Path(directory) / name
            if path.is_symlink():
                # Framework aliases are allowed only when contained in this app.
                if not path.resolve(strict=True).is_relative_to(app):
                    raise ValueError("code symlink escapes the bundle")
                continue
            with path.open("rb") as file:
                if file.read(4) in MACHO:
                    targets.append(path)
        for name in dirs:
            if name.endswith((".framework", ".xpc", ".app", ".bundle")):
                targets.append(Path(directory) / name)
    return sorted(targets, key=lambda p: (-len(p.parts), str(p)))


def sign(app, identity=None, *, adhoc_proof=False):
    macos()
    identity = "-" if adhoc_proof else identity or os.environ.get("PM_CODESIGN_IDENTITY")
    if not adhoc_proof and (not identity or identity.startswith("-")):
        raise ValueError("set PM_CODESIGN_IDENTITY to a Developer ID identity in Keychain")
    app = Path(app).resolve(strict=True)
    if adhoc_proof:
        info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
        if not re.fullmatch(r"com\.powermove\.(?:signature-proof|native\.dev)(?:\.[A-Za-z0-9-]+)*",
                            info.get("CFBundleIdentifier", "")):
            raise ValueError("ad-hoc proof signing is limited to proof and dev bundles")
    timestamp = "--timestamp=none" if adhoc_proof else "--timestamp"
    # Ad-hoc code has no Team ID, so the hardened runtime's library validation
    # would refuse the app's own dylibs and Sparkle; Developer ID always has it.
    runtime = [] if adhoc_proof else ["--options", "runtime"]
    proof_requirement = ["--requirements", '=designated => identifier "' + info["CFBundleIdentifier"] + '"'] if adhoc_proof else []
    if not (app / "Contents" / "Resources" / "Assets.car").is_file():
        raise ValueError("compile the asset catalog before distribution signing")
    # Validate identity availability without printing Keychain contents.
    for target in nested_code(app):
        if target == app / "Contents" / "MacOS" / EXECUTABLE:
            continue  # app signing seals its executable with the main entitlements
        run(["codesign", "--force", timestamp, *runtime, "--sign", identity,
             "--preserve-metadata=identifier,entitlements", target])
    run(["codesign", "--force", timestamp, *runtime, "--sign", identity,
         "--entitlements", HERE / "entitlements" / "app.plist", *proof_requirement, app])
    run(["codesign", "--verify", "--deep", "--strict", "--all-architectures", app])


def dmg(app, destination):
    macos()
    app = Path(app).resolve(strict=True)
    if app.suffix != ".app":
        raise ValueError("expected .app")
    destination = new_destination(destination)
    if destination.suffix != ".dmg":
        raise ValueError("expected .dmg destination")
    run(["codesign", "--verify", "--deep", "--strict", "--all-architectures", app])
    # An unstyled filesystem image; no Finder layout, background, window or copy choices.
    # Interrupted output is retained. No -ov flag and no recursive removal.
    staging = new_destination(destination.with_suffix(".dmg-staging"))
    staging.mkdir()
    shutil.copytree(app, staging / app.name, symlinks=True)
    run(["hdiutil", "create", "-format", "UDZO", "-fs", "HFS+", "-volname", "Powermove",
         "-srcfolder", staging, destination])
    return destination


def transition_zip(app, destination):
    macos()
    app = Path(app).resolve(strict=True)
    destination = new_destination(destination)
    if app.suffix != ".app" or destination.suffix != ".zip":
        raise ValueError("expected an app and a new ZIP destination")
    run(["codesign", "--verify", "--deep", "--strict", "--all-architectures", app])
    run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, destination])
    return destination


def notarize(artifact, profile=None):
    """Explicit local operator command only; never invoked by bundle/sign/dmg."""
    macos()
    artifact = Path(artifact).resolve(strict=True)
    if artifact.suffix not in (".dmg", ".app"):
        raise ValueError("notarize/staple a local app or DMG")
    # CI: an App Store Connect API key file (PM_NOTARY_KEY, _KEY_ID, _ISSUER).
    # A local operator: an existing notarytool Keychain profile.
    api = [os.environ.get(name) for name in ("PM_NOTARY_KEY", "PM_NOTARY_KEY_ID", "PM_NOTARY_ISSUER")]
    profile = profile or os.environ.get("PM_NOTARY_KEYCHAIN_PROFILE")
    if all(api) and not profile:
        if not Path(api[0]).is_file():
            raise ValueError("PM_NOTARY_KEY must name the API key file")
        credentials = ["--key", api[0], "--key-id", api[1], "--issuer", api[2]]
    elif profile and not profile.startswith("-"):
        credentials = ["--keychain-profile", profile]
    else:
        raise ValueError("set PM_NOTARY_KEY/_KEY_ID/_ISSUER or PM_NOTARY_KEYCHAIN_PROFILE")
    submission = artifact
    if artifact.suffix == ".app":
        submission = transition_zip(artifact, artifact.with_suffix(".notary.zip"))
    run(["xcrun", "notarytool", "submit", submission, *credentials, "--wait"])
    run(["xcrun", "stapler", "staple", artifact])
    run(["xcrun", "stapler", "validate", artifact])
    if artifact.suffix == ".app":
        run(["spctl", "--assess", "--type", "execute", artifact])
    else:
        run(["spctl", "--assess", "--type", "open", "--context", "context:primary-signature", artifact])


def verify_sparkle_archive(archive):
    """Verify the reviewed upstream artifact before extracting outside this repo."""
    pin = json.loads((HERE / "sparkle-pin.json").read_text())
    digest = hashlib.sha256()
    with Path(archive).open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != pin["sha256"]:
        raise ValueError("Sparkle archive does not match the reviewed SHA-256 pin")


# V1 (Electron, electron-updater 6.8.3 + Squirrel.Mac) installs an update only
# when the new bundle satisfies the running app's designated requirement, read
# from the signed V1 1.3.2 executable. V2 stable keeps its identifier and Team.
V1_REQUIREMENT = HERE / "v1-designated-requirement.txt"
# electron-updater compares minimumSystemVersion with os.release(), the Darwin
# kernel version, not the macOS version: macOS 14 is Darwin 23.
DARWIN = {"14.0": "23.0.0"}
ZIP_NAME = "Powermove-{version}-mac-arm64.zip"


def v1_requirement():
    text = V1_REQUIREMENT.read_text().strip()
    if not text.startswith('designated => identifier "' + IDS["cutover"] + '" '):
        raise ValueError("V1 requirement must name the production identifier")
    return text.removeprefix("designated => ")


def continuity(app, run=run):
    """The checks Squirrel.Mac makes before it swaps V1 for this bundle: the same
    bundle identifier (it picks the update bundle by identifier), a valid nested
    signature that satisfies V1's designated requirement, and an arm64 slice."""
    macos()
    app = Path(app).resolve(strict=True)
    metadata = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    if metadata.get("CFBundleIdentifier") != IDS["cutover"]:
        raise ValueError("V1 updates only into the production identifier")
    executable = app / "Contents/MacOS" / metadata["CFBundleExecutable"]
    archs = subprocess.run(["lipo", "-archs", str(executable)], check=True,
                           capture_output=True, text=True).stdout.split()
    if "arm64" not in archs:
        raise ValueError("V1 runs on Apple silicon only; the update needs arm64")
    run(["codesign", "--verify", "--deep", "--strict", "--all-architectures",
         "-R", "=" + v1_requirement(), app])


def openssl():
    """OpenSSL 3 or later (Ed25519 raw verification); macOS's LibreSSL lacks it."""
    for candidate in (os.environ.get("PM_OPENSSL"), "/opt/homebrew/opt/openssl@3/bin/openssl", "openssl"):
        if candidate and shutil.which(candidate):
            version = subprocess.run([candidate, "version"], capture_output=True, text=True).stdout
            if re.match(r"OpenSSL (?:[3-9]|\d\d)\.", version):
                return candidate
    raise ValueError("OpenSSL 3 or later is required to verify EdDSA signatures")


def verify_ed25519(path, signature, key):
    """Checks a Sparkle signature against the public key the app embeds, with an
    implementation independent of Sparkle's sign_update."""
    with tempfile.TemporaryDirectory() as temp:
        temp = Path(temp)
        (temp / "key.der").write_bytes(bytes.fromhex("302a300506032b6570032100") + base64.b64decode(public_key(key)))
        (temp / "signature").write_bytes(base64.b64decode(signature, validate=True))
        result = subprocess.run([openssl(), "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
                                 "-inkey", temp / "key.der", "-rawin", "-in", path,
                                 "-sigfile", temp / "signature"], capture_output=True)
    if result.returncode:
        raise ValueError("EdDSA signature does not match the app's public key")


def sparkle_sign(sign_update, key_file, path, *extra):
    result = subprocess.run([str(sign_update), "--ed-key-file", str(key_file), *extra, str(path)],
                            check=True, capture_output=True, text=True)
    return result.stdout.strip()


def feeds(archive, destination, version, build, tag, repo, sign_update, key_file, key,
          minimum="14.0", notes=None, date=None, download_base=None):
    """Writes the release's three feeds beside nothing else, from one signed ZIP:
    latest-mac.yml and beta-mac.yml for V1's electron-updater (stable and beta
    V1 users), and appcast.xml for V2's Sparkle, EdDSA-signed (archive and feed)."""
    archive = Path(archive).resolve(strict=True)
    if archive.name != ZIP_NAME.format(version=version):
        raise ValueError("expected " + ZIP_NAME.format(version=version))
    if not re.fullmatch(r"v" + re.escape(version), tag) or not re.fullmatch(r"[\w.-]+/[\w.-]+", repo):
        raise ValueError("tag must be v<version> and repo owner/name")
    info("cutover", version, build)  # the bundle's version rules
    destination = new_destination(destination)
    destination.mkdir(parents=True)
    data = archive.read_bytes()
    sha512 = base64.b64encode(hashlib.sha512(data).digest()).decode()
    size = len(data)
    date = date or datetime.now(timezone.utc)
    stamp = date.strftime("%Y-%m-%dT%H:%M:%S.") + f"{date.microsecond // 1000:03d}Z"
    yml = (f"version: {version}\nfiles:\n  - url: {archive.name}\n    sha512: {sha512}\n"
           f"    size: {size}\nminimumSystemVersion: {DARWIN[minimum]}\npath: {archive.name}\n"
           f"sha512: {sha512}\nreleaseDate: '{stamp}'\n")
    for channel in ("latest", "beta"):
        (destination / f"{channel}-mac.yml").write_text(yml)
    signature = sparkle_sign(sign_update, key_file, archive, "-p")
    verify_ed25519(archive, signature, key)
    # A tagged (immutable) download; a local proof serves the archive itself.
    base = download_base or f"https://github.com/{repo}/releases/download/{tag}"
    if download_base and urlsplit(download_base).hostname not in LOOPBACK:
        raise ValueError("only a loopback proof may override the download location")
    url = f"{base.rstrip('/')}/{archive.name}"
    text = Path(notes).read_text() if notes else "Fixes and improvements."
    description = escape(text) if "]]>" in text else "<![CDATA[" + text + "]]>"
    appcast = f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Powermove</title>
    <item>
      <title>Powermove {escape(version)}</title>
      <pubDate>{format_datetime(date)}</pubDate>
      <sparkle:version>{escape(build)}</sparkle:version>
      <sparkle:shortVersionString>{escape(version)}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{minimum}</sparkle:minimumSystemVersion>
      <description sparkle:format="markdown">{description}</description>
      <enclosure url="{escape(url)}" length="{size}" type="application/octet-stream" sparkle:edSignature="{signature}"/>
    </item>
  </channel>
</rss>
"""
    (destination / "appcast.xml").write_text(appcast)
    sparkle_sign(sign_update, key_file, destination / "appcast.xml")  # embeds the feed signature
    return destination


POSTHOG_PROJECT = "766251"
POSTHOG_APP_HOST = "https://us.posthog.com"


def symbols(executable, destination, version, build, run=run):
    """Crash symbols for PostHog error tracking: a dSYM of the release binary
    (the Rust symbol set, matched by its Mach-O UUID), uploaded only when a
    `POSTHOG_CLI_API_KEY` is in the environment (a release secret). Without
    one the dSYM is still written and the upload is skipped; `POSTHOG_CLI_DRY_RUN=1`
    checks the upload command without contacting PostHog."""
    macos()
    executable = native_file(executable)
    destination = new_destination(destination)
    destination.mkdir(parents=True)
    run(["dsymutil", executable, "-o", destination / (executable.name + ".dSYM")])
    dry = os.environ.get("POSTHOG_CLI_DRY_RUN") == "1"
    if not os.environ.get("POSTHOG_CLI_API_KEY") and not dry:
        print("No POSTHOG_CLI_API_KEY: the dSYM is written; the upload is skipped.")
        return
    os.environ.setdefault("POSTHOG_CLI_HOST", POSTHOG_APP_HOST)
    os.environ.setdefault("POSTHOG_CLI_PROJECT_ID", POSTHOG_PROJECT)
    run(["posthog-cli"] + (["--dry-run"] if dry else []) + [
        "dsym", "upload", "--directory", destination,
        "--main-dsym", executable.name + ".dSYM",
        "--release-name", "powermove", "--release-version", version, "--build", build,
        "--skip-on-conflict"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("bundle")
    create.add_argument("--executable", required=True, type=Path)
    create.add_argument("--destination", required=True, type=Path)
    create.add_argument("--channel", choices=IDS, required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--build", required=True)
    create.add_argument("--sparkle-framework", type=Path)
    create.add_argument("--sparkle-feed")
    create.add_argument("--sparkle-key")
    create.add_argument("--glsl-compiler", type=Path)
    create.add_argument("--claude-runtime", type=Path, help="The pinned native claude binary")
    create.add_argument("--codex-vendor", type=Path, help="The pinned vendor/<triple> tree")
    create.add_argument("--transcribe-runtime", type=Path, help="transcribe.cpp contract.json + dylibs")
    create.add_argument("--bundle-id", help="dev bundles only: com.powermove.native.dev.<name>")
    create.add_argument("--layout-only", action="store_true", help="Linux proof only; output cannot be distribution-signed")
    symbol_set = commands.add_parser("symbols")
    symbol_set.add_argument("--executable", required=True, type=Path)
    symbol_set.add_argument("--destination", required=True, type=Path)
    symbol_set.add_argument("--version", required=True)
    symbol_set.add_argument("--build", required=True)
    feed = commands.add_parser("feeds")
    feed.add_argument("--zip", required=True, type=Path)
    feed.add_argument("--destination", required=True, type=Path)
    feed.add_argument("--version", required=True)
    feed.add_argument("--build", required=True)
    feed.add_argument("--tag", required=True)
    feed.add_argument("--repo", required=True)
    feed.add_argument("--sign-update", required=True, type=Path, help="Sparkle's bin/sign_update")
    feed.add_argument("--ed-key-file", required=True, type=Path, help="Sparkle EdDSA private key file")
    feed.add_argument("--public-key", required=True, help="SUPublicEDKey the app embeds")
    feed.add_argument("--notes", type=Path, help="Markdown release notes")
    feed.add_argument("--download-base", help="local proofs only: a loopback URL serving the ZIP")
    commands.add_parser("continuity").add_argument("artifact", type=Path)
    verify = commands.add_parser("verify-sparkle-archive")
    verify.add_argument("artifact", type=Path)
    for name in ("sign", "dmg", "transition-zip", "notarize"):
        command = commands.add_parser(name)
        command.add_argument("artifact", type=Path)
        if name in ("dmg", "transition-zip"):
            command.add_argument("destination", type=Path)
        if name == "sign":
            command.add_argument("--adhoc", action="store_true",
                                 help="local dry run: ad-hoc sign a proof/dev bundle")
    args = parser.parse_args()
    if args.command == "bundle":
        bundle(args.executable, args.destination, args.channel, args.version, args.build,
               args.sparkle_framework, args.sparkle_feed, args.sparkle_key,
               not args.layout_only, args.glsl_compiler, args.claude_runtime, args.codex_vendor,
               args.transcribe_runtime, args.bundle_id)
    elif args.command == "feeds":
        feeds(args.zip, args.destination, args.version, args.build, args.tag, args.repo,
              args.sign_update, args.ed_key_file, args.public_key, notes=args.notes,
              download_base=args.download_base)
    elif args.command == "continuity":
        continuity(args.artifact)
    elif args.command == "symbols":
        symbols(args.executable, args.destination, args.version, args.build)
    elif args.command == "verify-sparkle-archive":
        verify_sparkle_archive(args.artifact)
    elif args.command == "sign":
        sign(args.artifact, adhoc_proof=args.adhoc)
    elif args.command == "dmg":
        dmg(args.artifact, args.destination)
    elif args.command == "transition-zip":
        transition_zip(args.artifact, args.destination)
    else:
        notarize(args.artifact)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        # Do not forward command argv, identity selectors or notarization responses.
        print(f"Distribution operation failed ({type(error).__name__}); retained local outputs.", file=sys.stderr)
        sys.exit(1)
