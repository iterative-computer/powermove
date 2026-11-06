#!/usr/bin/env python3
"""Headless synthetic signature proof. A real candidate uses scripts/prove-signed-build.py.
No application execution, Keychain identity lookup, notarization submission or upload.
Outputs can contain private paths and belong outside Git in a new directory.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent

def run(argv, *, accepts_failure=False):
    result = subprocess.run([str(a) for a in argv], capture_output=True)
    if not accepts_failure and result.returncode:
        raise RuntimeError("proof subprocess failed: " + str(argv[0]))
    return result

def output_root(path):
    path = Path(path)
    if not path.is_absolute() or path.exists() or path.is_symlink():
        raise ValueError("output must be a new absolute private directory")
    parent = path.parent.resolve(strict=True)
    if parent != path.parent:
        raise ValueError("canonicalize the output parent first")
    path.mkdir(mode=0o700)
    return path

def require(app, requirement):
    return run(["codesign", "--verify", "--deep", "--strict", "--all-architectures", "-R", "=" + requirement.removeprefix("="), app], accepts_failure=True).returncode == 0

def synthetic(root):
    spec = importlib.util.spec_from_file_location("pm_package", HERE / "package.py")
    package = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(package)
    source = root / "synthetic.c"
    source.write_text("int main(void) { return 0; }\n")
    binary = root / "fixture"
    run(["xcrun", "clang", "-arch", "arm64", "-arch", "x86_64", "-mmacosx-version-min=14.0", source, "-o", binary])
    apps = []
    checks = {}
    for name, identifier, version in [("old", "com.powermove.signature-proof", "1.0.0"), ("new", "com.powermove.signature-proof", "2.0.0"), ("wrong", "com.powermove.signature-proof.wrong", "2.0.0")]:
        app = root / (name + ".app")
        contents = app / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Helpers").mkdir()
        resources = contents / "Resources"
        resources.mkdir()
        shutil.copy2(binary, contents / "MacOS/powermove-native")
        shutil.copy2(binary, contents / "Helpers/proof-helper")
        (contents / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": identifier, "CFBundleExecutable": "powermove-native", "CFBundlePackageType": "APPL", "CFBundleVersion": version, "CFBundleShortVersionString": version, "LSMinimumSystemVersion": "14.0"}))
        catalog = package.NATIVE / "bundle/Assets.xcassets"
        run(["xcrun", "actool", "--compile", resources, "--platform", "macosx", "--minimum-deployment-target", "14.0", "--accent-color", "AccentColor", "--output-partial-info-plist", resources / "asset-info.plist", catalog])
        package.sign(app, adhoc_proof=True)
        checks[name + "_nested_signature"] = require(app, 'identifier "' + identifier + '"')
        apps.append(app)
    requirement = 'identifier "com.powermove.signature-proof"'
    checks["same_requirement_new_version"] = require(apps[1], requirement)
    checks["wrong_requirement_rejected"] = not require(apps[2], requirement)
    # Every slice must be present, signed and sealed. Never execute either slice.
    checks["both_architectures"] = all(set(run(["lipo", "-archs", app / "Contents/MacOS/powermove-native"]).stdout.decode().split()) == {"arm64", "x86_64"} for app in apps)
    entitlement_data = run(["codesign", "-d", "--entitlements", ":-", apps[1]]).stdout
    entitlements = plistlib.loads(entitlement_data)
    checks["only_reviewed_jit_entitlement"] = entitlements == {"com.apple.security.cs.allow-jit": True}
    damaged = apps[1] / "Contents/Helpers/proof-helper"
    with damaged.open("ab") as f:
        f.write(b"synthetic tamper")
    checks["nested_tamper_rejected"] = not require(apps[1], requirement)
    return {"schema": 1, "mode": "ad-hoc-synthetic", "checks": checks,
            "passed": sum(checks.values()), "failed": sum(not v for v in checks.values()),
            "limits": ["No Developer ID/Team continuity proof", "No notarization/Gatekeeper proof", "No application/helper execution", "No updater replacement/power-loss proof", "No Keychain proof"]}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["adhoc"])
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "darwin": raise ValueError("requires macOS")
    root = output_root(args.output)
    report = synthetic(root)
    (root / "proof.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"{report['passed']} signature checks passed / {report['failed']} failed ({report['mode']})")
    return bool(report["failed"])

if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, RuntimeError):
        print("Signature proof stopped; private partial outputs retained.", file=sys.stderr)
        sys.exit(1)
