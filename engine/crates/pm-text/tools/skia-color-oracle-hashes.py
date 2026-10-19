"""Regenerates rgbaHashes in tests/fixtures/skia-analytic-masks.json.

Usage: python3 -I skia-color-oracle-hashes.py FIXTURE ORACLE
ORACLE is skia-color-oracle.cpp built against the pinned Skia revision and
configuration named in the fixture. Each solidLayers entry is drawn in order
into a cleared N32 premul bitmap; the hash is FNV-1a 64 over its RGBA bytes.
"""
import json
import os
import subprocess
import sys
import tempfile


def fnv1a64(data):
    h = 14695981039346656037
    for v in data:
        h = ((h ^ v) * 1099511628211) & 0xFFFFFFFFFFFFFFFF
    return h


def main(fixture_path, oracle):
    fixture = json.load(open(fixture_path))
    with tempfile.TemporaryDirectory() as tmp:
        src, dst = os.path.join(tmp, "in.txt"), os.path.join(tmp, "out.rgba")
        for case in fixture["cases"]:
            hashes = {}
            for name, colors in fixture["solidLayers"].items():
                lines = [f"{case['width']} {case['height']} 0 0"]
                for color in colors:
                    for op in case["ops"]:
                        lines.append(" ".join([op[0]] + [repr(float(v)) for v in op[1:]]))
                    lines.append("D " + " ".join(repr(v / 255) for v in color))
                with open(src, "w") as f:
                    f.write("\n".join(lines) + "\n")
                subprocess.check_call([oracle, src, dst])
                with open(dst, "rb") as f:
                    hashes[name] = fnv1a64(f.read())
            case["rgbaHashes"] = hashes
    with open(fixture_path, "w") as f:
        f.write(json.dumps(fixture, indent=2) + "\n")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
