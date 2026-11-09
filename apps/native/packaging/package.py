#!/usr/bin/env python3
"""Local distribution operations only. No upload, feed publish, release or tag API.

Python is build tooling, never bundled. macOS commands are injectable for Linux
contract tests. Destination paths must be new; existing bundles are never removed.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
from urllib.parse import urlsplit

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


def config(path, channel):
    value = json.loads(Path(path).read_text())
    if set(value) != {"version", "enabled", "feedURL", "publicKey", "channel"}:
        raise ValueError("unknown/missing update configuration fields")
    if value["version"] != 1 or type(value["enabled"]) is not bool:
        raise ValueError("unsupported update configuration")
    if value["channel"] != channel:
        raise ValueError("update channel mismatch")
    if value["enabled"]:
        https(value["feedURL"])
        if urlsplit(value["feedURL"]).path.endswith(("latest-mac.yml", "beta-mac.yml")):
            raise ValueError("legacy Electron feed cannot deliver native policy")
        public_key(value["publicKey"])
    return value


def info(channel, version, build, sparkle_feed=None, sparkle_key=None):
    if channel not in IDS or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("explicit channel and numeric macOS bundle version required")
    if not re.fullmatch(r"[1-9]\d*(?:\.\d+){0,2}", build):
        raise ValueError("invalid bundle build number")
    if bool(sparkle_feed) != bool(sparkle_key):
        raise ValueError("Sparkle feed and public key must be provided together")
    value = {
        "CFBundleName": "Powermove", "CFBundleDisplayName": "Powermove",
        "CFBundleIdentifier": IDS[channel], "CFBundleExecutable": "powermove-native",
        "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version,
        "CFBundleVersion": build, "LSMinimumSystemVersion": "14.0",
        "LSApplicationCategoryType": "public.app-category.graphics-design",
        "NSAccentColorName": "AccentColor", "NSHighResolutionCapable": True,
        "NSPrincipalClass": "NSApplication", "CFBundleIconFile": "icon.icns",
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
        value.update(SUFeedURL=https(sparkle_feed), SUPublicEDKey=public_key(sparkle_key),
                     SUEnableAutomaticChecks=False, SUAutomaticallyUpdate=False)
    return value


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


HELPERS = ("pm-store-helper", "pm-glsl-compiler")


def bundle(executable, destination, channel, version, build, update_config,
           sparkle_framework=None, sparkle_feed=None, sparkle_key=None, compile_assets=True,
           glsl_compiler=None, claude_runtime=None, codex_vendor=None):
    metadata = info(channel, version, build, sparkle_feed, sparkle_key)
    updates = config(update_config, "cutover" if channel == "dev" else channel)
    if updates["enabled"] and not all((sparkle_framework, sparkle_feed, sparkle_key)):
        raise ValueError("enabled updates require the reviewed Sparkle framework/feed/key")
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
    shutil.copy2(executable, contents / "MacOS" / "powermove-native")
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
    # The layout provider discovery reads: <name>/bin/<name> under Resources.
    if claude_runtime:
        (resources / "claude" / "bin").mkdir(parents=True)
        shutil.copy2(native_file(claude_runtime), resources / "claude" / "bin" / "claude")
    if codex_vendor:
        codex_vendor = Path(codex_vendor)
        native_file(codex_vendor / "bin" / "codex")
        shutil.copytree(codex_vendor, resources / "codex", symlinks=True)
    with (contents / "Info.plist").open("wb") as file:
        plistlib.dump(metadata, file, sort_keys=True)
    (contents / "PkgInfo").write_bytes(b"APPL????")
    for name in ("icon.icns", "document.icns"):
        shutil.copy2(NATIVE / "bundle" / name, resources / name)
    # Source catalog is owned by pm-design/Opus; never generate new colour values.
    shutil.copytree(NATIVE / "bundle" / "Assets.xcassets", resources / "Assets.xcassets")
    if compile_assets:
        run(["xcrun", "actool", "--compile", resources, "--platform", "macosx",
             "--minimum-deployment-target", "14.0", "--accent-color", "AccentColor",
             "--output-partial-info-plist", resources / "asset-info.plist",
             resources / "Assets.xcassets"])
    (resources / "update-config.json").write_text(json.dumps(updates, indent=2) + "\n")
    if sparkle_framework:
        (contents / "Frameworks").mkdir()
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
        if not re.fullmatch(r"com\.powermove\.signature-proof(?:\.[A-Za-z0-9-]+)*", info.get("CFBundleIdentifier", "")):
            raise ValueError("ad-hoc proof signing is limited to synthetic proof bundles")
    timestamp = "--timestamp=none" if adhoc_proof else "--timestamp"
    proof_requirement = ["--requirements", '=designated => identifier "' + info["CFBundleIdentifier"] + '"'] if adhoc_proof else []
    if not (app / "Contents" / "Resources" / "Assets.car").is_file():
        raise ValueError("compile the asset catalog before distribution signing")
    # Validate identity availability without printing Keychain contents.
    for target in nested_code(app):
        if target == app / "Contents" / "MacOS" / "powermove-native":
            continue  # app signing seals its executable with the main entitlements
        run(["codesign", "--force", timestamp, "--options", "runtime", "--sign", identity,
             "--preserve-metadata=identifier,entitlements", target])
    run(["codesign", "--force", timestamp, "--options", "runtime", "--sign", identity,
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
    profile = profile or os.environ.get("PM_NOTARY_KEYCHAIN_PROFILE")
    if not profile or profile.startswith("-"):
        raise ValueError("set PM_NOTARY_KEYCHAIN_PROFILE to an existing notarytool Keychain profile")
    submission = artifact
    if artifact.suffix == ".app":
        submission = transition_zip(artifact, artifact.with_suffix(".notary.zip"))
    run(["xcrun", "notarytool", "submit", submission, "--keychain-profile", profile, "--wait"])
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("bundle")
    create.add_argument("--executable", required=True, type=Path)
    create.add_argument("--destination", required=True, type=Path)
    create.add_argument("--channel", choices=IDS, required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--build", required=True)
    create.add_argument("--update-config", default=HERE / "update-config.json", type=Path)
    create.add_argument("--sparkle-framework", type=Path)
    create.add_argument("--sparkle-feed")
    create.add_argument("--sparkle-key")
    create.add_argument("--glsl-compiler", type=Path)
    create.add_argument("--claude-runtime", type=Path, help="The pinned native claude binary")
    create.add_argument("--codex-vendor", type=Path, help="The pinned vendor/<triple> tree")
    create.add_argument("--layout-only", action="store_true", help="Linux proof only; output cannot be distribution-signed")
    verify = commands.add_parser("verify-sparkle-archive")
    verify.add_argument("artifact", type=Path)
    for name in ("sign", "dmg", "transition-zip", "notarize"):
        command = commands.add_parser(name)
        command.add_argument("artifact", type=Path)
        if name in ("dmg", "transition-zip"):
            command.add_argument("destination", type=Path)
    args = parser.parse_args()
    if args.command == "bundle":
        bundle(args.executable, args.destination, args.channel, args.version, args.build,
               args.update_config, args.sparkle_framework, args.sparkle_feed, args.sparkle_key,
               not args.layout_only, args.glsl_compiler, args.claude_runtime, args.codex_vendor)
    elif args.command == "verify-sparkle-archive":
        verify_sparkle_archive(args.artifact)
    elif args.command == "sign":
        sign(args.artifact)
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
