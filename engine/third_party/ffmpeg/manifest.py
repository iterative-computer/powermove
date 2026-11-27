"""Writes out/manifest.json for Powermove's FFmpeg build (called by build.sh).

The manifest records what was built and from what: pinned source versions and
checksums, the exact FFmpeg configure flags, the toolchain, and a checksum,
install name, and dependency list for every shipped binary. It carries no
timestamps or local paths, so two identical builds produce identical manifests.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def otool(flag, path):
    return subprocess.run(["otool", flag, path], check=True, capture_output=True, text=True).stdout


def linked_libraries(path):
    """`otool -L` entries, without the file's own install name."""
    lines = otool("-L", path).splitlines()[1:]
    names = [line.strip().split(" (compatibility")[0] for line in lines if line.strip()]
    own = install_name(path)
    return [name for name in names if name != own]


def install_name(path):
    lines = otool("-D", path).splitlines()[1:]
    return lines[0].strip() if lines else None


def rpaths(path):
    out, result, pending = otool("-l", path).splitlines(), [], False
    for line in out:
        line = line.strip()
        if line == "cmd LC_RPATH":
            pending = True
        elif pending and line.startswith("path "):
            result.append(line[len("path "):].split(" (offset")[0])
            pending = False
    return result


def min_os(path):
    out = otool("-l", path).splitlines()
    for index, line in enumerate(out):
        if line.strip() == "cmd LC_BUILD_VERSION":
            for follow in out[index + 1:index + 6]:
                follow = follow.strip()
                if follow.startswith("minos "):
                    return follow.split()[1]
    return None


def read_tsv(path):
    with open(path, encoding="utf-8") as handle:
        return [line.rstrip("\n").split("\t") for line in handle if line.strip()]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--sources", required=True)
    parser.add_argument("--toolchain", required=True)
    parser.add_argument("--configure-flags", required=True)
    parser.add_argument("--buildconf", required=True)
    parser.add_argument("--ffmpeg-version", required=True)
    args = parser.parse_args()

    out = os.path.abspath(args.out)
    ffmpeg = os.path.join(out, "bin", "ffmpeg")
    license_text = subprocess.run([ffmpeg, "-hide_banner", "-L"], check=True, capture_output=True, text=True).stdout
    license_statement = " ".join(license_text.strip().split("\n\n")[0].split())
    with open(args.buildconf, encoding="utf-8") as handle:
        buildconf = [line.strip() for line in handle if line.strip().startswith("--")]

    sources = [
        {"name": name, "version": version, "sha256": digest, "url": url, "license": spdx, "role": role}
        for name, version, digest, url, spdx, role in read_tsv(args.sources)
    ]
    toolchain = {key: value for key, value in read_tsv(args.toolchain)}

    outputs = []
    for folder in ("bin", "lib"):
        root = os.path.join(out, folder)
        for name in sorted(os.listdir(root)):
            path = os.path.join(root, name)
            if os.path.islink(path) or not os.path.isfile(path):
                continue
            if folder == "lib" and not name.endswith(".dylib"):
                continue
            outputs.append({
                "path": f"{folder}/{name}",
                "sha256": sha256(path),
                "bytes": os.path.getsize(path),
                "install_name": install_name(path),
                "rpaths": rpaths(path),
                "min_macos": min_os(path),
                "links": linked_libraries(path),
            })
    symlinks = {
        f"lib/{name}": os.readlink(os.path.join(out, "lib", name))
        for name in sorted(os.listdir(os.path.join(out, "lib")))
        if os.path.islink(os.path.join(out, "lib", name))
    }

    manifest = {
        "schema": 1,
        "target": "x86_64-apple-darwin" if os.environ.get("PM_FFMPEG_ARCH") == "x86_64" else "aarch64-apple-darwin",
        "ffmpeg": {
            "version": args.ffmpeg_version,
            "license": license_statement,
            "configure_flags": [flag for flag in args.configure_flags.split("\n") if flag],
            "buildconf": buildconf,
        },
        "sources": sources,
        "toolchain": toolchain,
        "outputs": outputs,
        "symlinks": symlinks,
    }
    with open(os.path.join(out, "manifest.json"), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
