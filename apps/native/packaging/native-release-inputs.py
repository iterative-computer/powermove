#!/usr/bin/env python3
"""Stage the pinned npm runtimes and bundled public update policy."""
import base64
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tarfile
from urllib.request import urlopen

from package import HERE, NATIVE, public_key


def stage_runtime(root, package, version, integrity, prefix, destination):
    archive = root / (destination.name + ".tgz")
    filename = package.rsplit("/", 1)[-1]
    url = f"https://registry.npmjs.org/{package}/-/{filename}-{version}.tgz"
    with urlopen(url, timeout=90) as response, archive.open("xb") as output:
        shutil.copyfileobj(response, output)
    digest = hashlib.sha512()
    with archive.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    actual = "sha512-" + base64.b64encode(digest.digest()).decode()
    if actual != integrity:
        raise ValueError("Pinned runtime SHA-512 mismatch")

    with tarfile.open(archive) as tar:
        if not prefix.endswith("/"):
            members = [(tar.getmember(prefix), destination)]
        else:
            members = [(member, destination / member.name[len(prefix):])
                       for member in tar.getmembers() if member.name.startswith(prefix)]
        for member, path in members:
            if not member.isfile() or not path.resolve().is_relative_to(root.resolve()):
                raise ValueError("Unexpected runtime archive entry")
            path.parent.mkdir(parents=True, exist_ok=True)
            with tar.extractfile(member) as source, path.open("xb") as output:
                shutil.copyfileobj(source, output)
            path.chmod(member.mode & 0o755)
    archive.unlink()


def main(stage):
    key_data = json.loads((HERE / "release" / "public-keys.json").read_text())
    policy_key = public_key(key_data["policyPublicKey"])
    if policy_key == public_key(key_data["sparklePublicKey"]):
        raise ValueError("The archive and policy must use independent keys")
    policy_url = "https://cloud.trypowermove.com/native/latest/policy.json"
    config = dict(version=1, enabled=True, feedURL=policy_url, publicKey=policy_key, channel="latest")
    (stage / "update-config.json").write_text(json.dumps(config) + "\n")

    pins = json.loads((NATIVE / "crates/agents/fixtures/runtime-pins.json").read_text())
    integrity = {entry["package"]: entry["integrity"] for entry in pins["pins"]}
    root = stage / "runtime"
    root.mkdir()
    claude = "@anthropic-ai/claude-code-darwin-arm64"
    codex = "@openai/codex-darwin-arm64"
    stage_runtime(root, claude, pins["claude"], integrity[claude], "package/claude", root / "claude")
    stage_runtime(root, "@openai/codex", pins["codex"] + "-darwin-arm64", integrity[codex],
                  "package/vendor/aarch64-apple-darwin/", root / "codex")
    # Same pinned macOS build as Electron's lockfile and the Rust ABI check.
    stage_runtime(root, "@transcribe-cpp/darwin-arm64-metal", "0.3.1",
                  "sha512-Fk6crfpnFmbelnDNIVJ8NzJbQHMwQIZcd3ZquVMTjNxPIoU1xqABKV5iKPcCnxtRk48g6QRDgH2tEIN8nTp7qw==",
                  "package/", root / "transcribe")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
