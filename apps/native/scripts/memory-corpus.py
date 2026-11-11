#!/usr/bin/env python3
"""Generate public, deterministic memory fixtures outside the repository.

python3 scripts/memory-corpus.py /tmp/pm-memory-corpus
pm-engine-service --memory-bench --fixture=/tmp/pm-memory-corpus/many-layers
Requires ffmpeg for the twenty distinct video sources. No third-party Python deps.
"""
import copy
import json
from pathlib import Path
import struct
import subprocess
import sys
import zlib


def write(path, value):
    path.write_text(json.dumps(value, separators=(",", ":")))


def png(path, width, height):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    # Compress rows incrementally: generating a large still never holds RGBA pixels.
    compressor = zlib.compressobj()
    data = bytearray()
    for y in range(height):
        row = b"\0" + bytes((y % 256, 90, 180, 255)) * width
        data.extend(compressor.compress(row))
    data.extend(compressor.flush())
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", data) + chunk(b"IEND", b""))


def generate(root):
    source = Path(__file__).parent.parent / "crates/engine-service/tests/fixtures/shapes.json"
    template = json.loads(source.read_text())
    for kind in ("many-layers", "long-history", "twenty-videos", "large-stills"):
        dest = root / kind
        dest.mkdir(parents=True, exist_ok=True)
        project = copy.deepcopy(template)
        project.update(w=1920, h=1080, dur=20, work=[0, 20], assets={})
        project["layers"] = []
        media = {}
        count = {"many-layers": 1000, "long-history": 1, "twenty-videos": 20, "large-stills": 9}[kind]
        for n in range(count):
            layer = copy.deepcopy(template["layers"][0])
            layer.update(id=f"L{n}", dur=20, name=f"Generated {n}")
            layer["p"]["position.x"]["v"] = 80 + n % 12 * 150
            layer["p"]["position.y"]["v"] = 80 + n % 8 * 120
            if kind in ("twenty-videos", "large-stills"):
                ty = "video" if kind == "twenty-videos" else "image"
                asset = f"A{n}"
                width, height = (1920, 1080) if ty == "video" else (3024, 4032)
                name = f"{asset}.mp4" if ty == "video" else f"{asset}.png"
                if ty == "video":
                    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i",
                                    f"color=c=0x{n * 700001 % 0xffffff:06x}:s={width}x{height}:r=30:d=20",
                                    "-c:v", "libx264", "-preset", "ultrafast", "-threads", "1", "-pix_fmt", "yuv420p", str(dest / name)], check=True)
                else:
                    png(dest / name, width, height)
                layer.update(type=ty, d={"asset": asset, "trim": 0, "speed": 1, "w": width, "h": height})
                layer["p"]["scale.x"]["v"] = 15
                layer["p"]["scale.y"]["v"] = 15
                project["assets"][asset] = {"id": asset, "type": ty, "name": name, "w": width, "h": height, "dur": 20}
                media[asset] = name
            project["layers"].append(layer)
        history = {"version": 1, "paths": "composition", "index": -1, "entries": []}
        if kind == "long-history":
            for n in range(1000):
                patch = {"path": ["comps", project["compId"], "layers", 0, "name"], "exists": True, "value": f"Edit {n} " + "x" * 8192}
                back = dict(patch, value=f"Edit {n - 1} " + "x" * 8192)
                history["entries"].append({"label": "Rename", "forward": [patch], "backward": [back]})
            history["index"] = 999
        write(dest / "project.pmv", {"v": "1.0.0", "proj": project, "history": history})
        write(dest / "media.json", media)


if __name__ == "__main__":
    generate(Path(sys.argv[1]).resolve())
