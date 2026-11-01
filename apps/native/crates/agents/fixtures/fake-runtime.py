#!/usr/bin/python3
"""Replay synthetic TS wire transcripts over real subprocess pipes."""
import json
import sys

with open(sys.argv[1], encoding="utf-8") as fixture:
    frames = json.load(fixture)["frames"]
for frame in frames:
    if frame["kind"] == "out":
        raw = sys.stdin.buffer.readline()
        if not raw:
            raise SystemExit("client closed before expected request")
        actual = json.loads(raw)
        if actual != frame["value"]:
            raise SystemExit("wire transcript differed")
    elif frame["kind"] == "in":
        data = (json.dumps(frame["value"], ensure_ascii=False) + "\n").encode()
        # Split every UTF-8 code point to exercise incremental transport framing.
        for byte in data:
            sys.stdout.buffer.write(bytes([byte]))
        sys.stdout.buffer.flush()
